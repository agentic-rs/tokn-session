use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use tokn_session_codex::{event::CodexLine, normalize::CodexNormalizer};
use tokn_session_core::{AgentEvent, Provider};
use tokn_session_pi::{event::PiEvent, normalize::PiNormalizer};

#[derive(Clone, Debug)]
pub struct ProviderRoot {
  pub provider: Provider,
  pub path: PathBuf,
}

impl ProviderRoot {
  pub fn new(provider: Provider, path: PathBuf) -> Self {
    Self { provider, path }
  }
}

#[derive(Debug)]
pub struct RelayEvent {
  pub path: PathBuf,
  pub topic: String,
  pub event: AgentEvent,
}

#[derive(Debug, Default)]
pub struct TailUpdate {
  pub events: Vec<RelayEvent>,
  pub warnings: Vec<String>,
}

pub struct SessionTailer {
  roots: Vec<ProviderRoot>,
  files: HashMap<PathBuf, FileState>,
  new_file_history: usize,
}

impl SessionTailer {
  pub fn initialize(
    roots: Vec<ProviderRoot>,
    replay: bool,
    new_file_history: usize,
  ) -> Result<(Self, TailUpdate), String> {
    let mut tailer = Self::prepare(roots, new_file_history)?;
    let update = tailer.start(replay)?;
    Ok((tailer, update))
  }

  pub(crate) fn prepare(roots: Vec<ProviderRoot>, new_file_history: usize) -> Result<Self, String> {
    let mut tailer = Self {
      roots,
      files: HashMap::new(),
      new_file_history,
    };
    let paths = tailer.discover_paths()?;
    for (path, provider) in paths {
      tailer.files.insert(path.clone(), FileState::open(path, provider)?);
    }
    Ok(tailer)
  }

  pub(crate) fn start(&mut self, replay: bool) -> Result<TailUpdate, String> {
    let mut update = TailUpdate::default();
    let new_file_history = self.new_file_history;
    for state in self.files.values_mut() {
      let mode = if replay {
        InitialRead::Replay
      } else if state.matches_initial_snapshot()? {
        InitialRead::Follow
      } else {
        InitialRead::Backfill(new_file_history)
      };
      let initial = read_initial(state, mode)?;
      update.append(initial);
    }
    Ok(update)
  }

  pub fn scan(&mut self) -> Result<TailUpdate, String> {
    let discovered = self.discover_paths()?;
    let mut update = TailUpdate::default();

    for (path, provider) in discovered {
      if !self.files.contains_key(&path) {
        self.add_file(
          path,
          provider,
          InitialRead::Backfill(self.new_file_history),
          &mut update,
        )?;
      }
    }

    let paths = self.files.keys().cloned().collect::<Vec<_>>();
    for path in paths {
      let Some(state) = self.files.get_mut(&path) else {
        continue;
      };
      if !path.exists() {
        self.files.remove(&path);
        continue;
      }
      let (mut appended, restarted) = state.read_appended(true)?;
      if restarted {
        retain_message_history(&mut appended.events, self.new_file_history);
      }
      update.append(appended);
    }

    Ok(update)
  }

  pub fn roots(&self) -> &[ProviderRoot] {
    &self.roots
  }

  fn add_file(
    &mut self,
    path: PathBuf,
    provider: Provider,
    mode: InitialRead,
    update: &mut TailUpdate,
  ) -> Result<(), String> {
    let mut state = FileState::open(path.clone(), provider)?;
    let initial = read_initial(&mut state, mode)?;
    update.append(initial);
    self.files.insert(path, state);
    Ok(())
  }

  fn discover_paths(&self) -> Result<Vec<(PathBuf, Provider)>, String> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for root in &self.roots {
      collect_jsonl_files(&root.path, root.provider, &mut seen, &mut paths)?;
    }
    Ok(paths)
  }
}

fn read_initial(state: &mut FileState, mode: InitialRead) -> Result<TailUpdate, String> {
  match mode {
    InitialRead::Follow => state.seed_at_eof(),
    InitialRead::Replay => Ok(state.read_appended(true)?.0),
    InitialRead::Backfill(message_count) => {
      let mut update = state.read_appended(true)?.0;
      retain_message_history(&mut update.events, message_count);
      Ok(update)
    }
  }
}

#[derive(Clone, Copy)]
enum InitialRead {
  Follow,
  Replay,
  Backfill(usize),
}

impl TailUpdate {
  fn append(&mut self, mut other: Self) {
    self.events.append(&mut other.events);
    self.warnings.append(&mut other.warnings);
  }
}

struct FileState {
  path: PathBuf,
  provider: Provider,
  identity: FileIdentity,
  initial_length: u64,
  offset: u64,
  pending: Vec<u8>,
  normalizer: SessionNormalizer,
}

impl FileState {
  fn open(path: PathBuf, provider: Provider) -> Result<Self, String> {
    let metadata = std::fs::metadata(&path).map_err(|err| format!("failed to inspect {}: {err}", path.display()))?;
    Ok(Self {
      path,
      provider,
      identity: file_identity(&metadata),
      initial_length: metadata.len(),
      offset: 0,
      pending: Vec::new(),
      normalizer: SessionNormalizer::new(provider),
    })
  }

  fn seed_at_eof(&mut self) -> Result<TailUpdate, String> {
    const MAX_SESSION_HEADER_LINES: usize = 64;

    let file = File::open(&self.path).map_err(|err| format!("failed to open {}: {err}", self.path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut update = TailUpdate::default();
    for _ in 0..MAX_SESSION_HEADER_LINES {
      line.clear();
      let bytes = reader
        .read_line(&mut line)
        .map_err(|err| format!("failed to read {}: {err}", self.path.display()))?;
      if bytes == 0 || !line.ends_with('\n') {
        break;
      }
      match self.normalizer.normalize_line(line.trim_end_matches(['\r', '\n'])) {
        Ok(events)
          if events
            .iter()
            .any(|event| matches!(event, AgentEvent::SessionStarted(_))) =>
        {
          break;
        }
        Ok(_) => {}
        Err(err) => update
          .warnings
          .push(format!("failed to seed {}: {err}", self.path.display())),
      }
    }

    self.offset = self.initial_length;
    self.pending = trailing_partial_line(&self.path, self.offset)?;
    Ok(update)
  }

  fn matches_initial_snapshot(&self) -> Result<bool, String> {
    let metadata =
      std::fs::metadata(&self.path).map_err(|err| format!("failed to inspect {}: {err}", self.path.display()))?;
    Ok(file_identity(&metadata) == self.identity && metadata.len() >= self.initial_length)
  }

  fn read_appended(&mut self, publish: bool) -> Result<(TailUpdate, bool), String> {
    let metadata =
      std::fs::metadata(&self.path).map_err(|err| format!("failed to inspect {}: {err}", self.path.display()))?;
    let identity = file_identity(&metadata);

    let mut should_publish = publish;
    let mut restarted = false;
    if identity != self.identity || metadata.len() < self.offset {
      self.identity = identity;
      self.offset = 0;
      self.pending.clear();
      self.normalizer = SessionNormalizer::new(self.provider);
      should_publish = true;
      restarted = true;
    }
    let length = metadata.len();
    if length == self.offset {
      return Ok((TailUpdate::default(), restarted));
    }

    let mut file = File::open(&self.path).map_err(|err| format!("failed to open {}: {err}", self.path.display()))?;
    file
      .seek(SeekFrom::Start(self.offset))
      .map_err(|err| format!("failed to seek {}: {err}", self.path.display()))?;
    let mut appended = Vec::new();
    file
      .read_to_end(&mut appended)
      .map_err(|err| format!("failed to read {}: {err}", self.path.display()))?;
    self.offset += appended.len() as u64;
    self.pending.extend(appended);

    let complete_length = self
      .pending
      .iter()
      .rposition(|byte| *byte == b'\n')
      .map(|index| index + 1)
      .unwrap_or(0);
    if complete_length == 0 {
      return Ok((TailUpdate::default(), restarted));
    }

    let complete = self.pending.drain(..complete_length).collect::<Vec<_>>();
    let mut update = TailUpdate::default();
    for raw_line in complete.split(|byte| *byte == b'\n') {
      let line = match std::str::from_utf8(raw_line) {
        Ok(line) if !line.trim().is_empty() => line,
        Ok(_) => continue,
        Err(err) => {
          update
            .warnings
            .push(format!("invalid UTF-8 in {}: {err}", self.path.display()));
          continue;
        }
      };

      match self.normalizer.normalize_line(line) {
        Ok(events) if should_publish => {
          update.events.extend(events.into_iter().map(|event| RelayEvent {
            topic: event_topic(self.provider, &self.path, &event),
            path: self.path.clone(),
            event,
          }));
        }
        Ok(_) => {}
        Err(err) => update
          .warnings
          .push(format!("failed to normalize {}: {err}", self.path.display())),
      }
    }
    Ok((update, restarted))
  }
}

fn retain_message_history(events: &mut Vec<RelayEvent>, message_count: usize) {
  if message_count == 0 {
    events.clear();
    return;
  }

  let message_indices = events
    .iter()
    .enumerate()
    .filter_map(|(index, event)| matches!(event.event, AgentEvent::Message(_)).then_some(index))
    .collect::<Vec<_>>();
  if message_indices.len() <= message_count {
    return;
  }

  let start = message_indices[message_indices.len() - message_count];
  events.drain(..start);
}

fn trailing_partial_line(path: &Path, length: u64) -> Result<Vec<u8>, String> {
  const CHUNK_SIZE: usize = 8 * 1024;

  if length == 0 {
    return Ok(Vec::new());
  }

  let mut file = File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
  let mut position = length;
  let mut chunks = Vec::new();
  loop {
    let chunk_length = position.min(CHUNK_SIZE as u64) as usize;
    position -= chunk_length as u64;
    file
      .seek(SeekFrom::Start(position))
      .map_err(|err| format!("failed to seek {}: {err}", path.display()))?;
    let mut chunk = vec![0; chunk_length];
    file
      .read_exact(&mut chunk)
      .map_err(|err| format!("failed to read {}: {err}", path.display()))?;

    if let Some(newline) = chunk.iter().rposition(|byte| *byte == b'\n') {
      chunks.push(chunk[(newline + 1)..].to_vec());
      break;
    }
    chunks.push(chunk);
    if position == 0 {
      break;
    }
  }

  chunks.reverse();
  Ok(chunks.into_iter().flatten().collect())
}

enum SessionNormalizer {
  Codex(CodexNormalizer),
  Pi(PiNormalizer),
}

impl SessionNormalizer {
  fn new(provider: Provider) -> Self {
    match provider {
      Provider::Codex => Self::Codex(CodexNormalizer::new()),
      Provider::Pi => Self::Pi(PiNormalizer::new()),
      Provider::OpenCode => unreachable!("relay only supports JSONL providers"),
    }
  }

  fn normalize_line(&mut self, line: &str) -> Result<Vec<AgentEvent>, String> {
    match self {
      Self::Codex(normalizer) => {
        let event: CodexLine = serde_json::from_str(line).map_err(|err| format!("invalid codex JSONL: {err}"))?;
        Ok(normalizer.normalize(event))
      }
      Self::Pi(normalizer) => {
        let event: PiEvent = serde_json::from_str(line).map_err(|err| format!("invalid pi JSONL: {err}"))?;
        Ok(normalizer.normalize(event))
      }
    }
  }
}

fn event_topic(provider: Provider, path: &Path, event: &AgentEvent) -> String {
  let provider = provider_name(provider);
  let session_id = event_session_id(event)
    .map(str::to_string)
    .unwrap_or_else(|| session_id_from_path(path));
  format!("{provider}.{session_id}")
}

fn provider_name(provider: Provider) -> &'static str {
  match provider {
    Provider::Codex => "codex",
    Provider::Pi => "pi",
    Provider::OpenCode => "opencode",
  }
}

fn event_session_id(event: &AgentEvent) -> Option<&str> {
  match event {
    AgentEvent::SessionStarted(event) => Some(&event.session_id),
    AgentEvent::ProviderChanged(event) => event.session_id.as_deref(),
    AgentEvent::Message(event) => event.session_id.as_deref(),
    AgentEvent::Reasoning(event) => event.session_id.as_deref(),
    AgentEvent::GoalUpdated(event) => event.session_id.as_deref(),
    AgentEvent::ToolCall(event) => event.session_id.as_deref(),
    AgentEvent::Error(event) => event.session_id.as_deref(),
    AgentEvent::Unknown(event) => event.session_id.as_deref(),
  }
}

fn session_id_from_path(path: &Path) -> String {
  path
    .file_stem()
    .and_then(|value| value.to_str())
    .unwrap_or("unknown")
    .rsplit(['-', '_'])
    .next()
    .unwrap_or("unknown")
    .to_string()
}

fn collect_jsonl_files(
  dir: &Path,
  provider: Provider,
  seen: &mut HashSet<PathBuf>,
  paths: &mut Vec<(PathBuf, Provider)>,
) -> Result<(), String> {
  if !dir.exists() {
    return Ok(());
  }
  if dir.is_file() {
    if is_jsonl(dir) {
      let path = dir.to_path_buf();
      if seen.insert(path.clone()) {
        paths.push((path, provider));
      }
    }
    return Ok(());
  }

  for entry in std::fs::read_dir(dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))? {
    let entry = entry.map_err(|err| format!("failed to read entry in {}: {err}", dir.display()))?;
    let path = entry.path();
    if path.is_dir() {
      collect_jsonl_files(&path, provider, seen, paths)?;
    } else if is_jsonl(&path) && seen.insert(path.clone()) {
      paths.push((path, provider));
    }
  }
  Ok(())
}

fn is_jsonl(path: &Path) -> bool {
  path.extension().and_then(|value| value.to_str()) == Some("jsonl")
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
  device: u64,
  inode: u64,
}

#[cfg(unix)]
fn file_identity(metadata: &std::fs::Metadata) -> FileIdentity {
  use std::os::unix::fs::MetadataExt;

  FileIdentity {
    device: metadata.dev(),
    inode: metadata.ino(),
  }
}

#[cfg(not(unix))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
  created: Option<std::time::SystemTime>,
}

#[cfg(not(unix))]
fn file_identity(metadata: &std::fs::Metadata) -> FileIdentity {
  FileIdentity {
    created: metadata.created().ok(),
  }
}

#[cfg(test)]
mod tests {
  use std::fs::OpenOptions;
  use std::io::Write;

  use tempfile::TempDir;
  use tokn_session_core::{AgentEvent, Provider};

  use super::{ProviderRoot, SessionTailer};

  #[test]
  fn starts_at_eof_but_seeds_pi_session_context() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("session_test.jsonl");
    std::fs::write(
      &path,
      concat!(
        "{\"type\":\"session\",\"id\":\"pi-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
        "{\"type\":\"message\",\"id\":\"old\",\"message\":{\"role\":\"user\",\"content\":\"old\"}}\n"
      ),
    )
    .unwrap();

    let (mut tailer, initial) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      false,
      3,
    )
    .unwrap();
    assert!(initial.events.is_empty());

    append(
      &path,
      "{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n",
    );
    let update = tailer.scan().unwrap();
    assert_eq!(update.events.len(), 1);
    assert_eq!(update.events[0].topic, "pi.pi-session");
    let AgentEvent::Message(message) = &update.events[0].event else {
      panic!("expected message");
    };
    assert_eq!(message.text, "done");
    assert_eq!(message.session_id.as_deref(), Some("pi-session"));
  }

  #[test]
  fn starts_at_eof_without_parsing_the_existing_history() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("session_test.jsonl");
    std::fs::write(
      &path,
      concat!(
        "{\"type\":\"session\",\"id\":\"pi-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
        "this historical line is deliberately invalid JSON\n",
        "{\"type\":\"message\",\"id\":\"old\",\"message\":{\"role\":\"user\",\"content\":\"old\"}}\n"
      ),
    )
    .unwrap();

    let (mut tailer, initial) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      false,
      3,
    )
    .unwrap();
    assert!(initial.events.is_empty());
    assert!(initial.warnings.is_empty());

    append(
      &path,
      "{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"user\",\"content\":\"new\"}}\n",
    );
    let update = tailer.scan().unwrap();
    assert_eq!(update.events.len(), 1);
    let AgentEvent::Message(message) = &update.events[0].event else {
      panic!("expected message");
    };
    assert_eq!(message.text, "new");
  }

  #[test]
  fn catches_appends_between_the_snapshot_and_follow_start() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("session_test.jsonl");
    std::fs::write(
      &path,
      "{\"type\":\"session\",\"id\":\"pi-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
    )
    .unwrap();
    let mut tailer =
      SessionTailer::prepare(vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())], 3).unwrap();

    append(
      &path,
      "{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"user\",\"content\":\"not lost\"}}\n",
    );
    assert!(tailer.start(false).unwrap().events.is_empty());

    let update = tailer.scan().unwrap();
    assert_eq!(update.events.len(), 1);
    let AgentEvent::Message(message) = &update.events[0].event else {
      panic!("expected message");
    };
    assert_eq!(message.text, "not lost");
  }

  #[test]
  fn backfills_files_replaced_before_follow_start() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("session_test.jsonl");
    std::fs::write(
      &path,
      "{\"type\":\"session\",\"id\":\"old-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
    )
    .unwrap();
    let mut tailer =
      SessionTailer::prepare(vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())], 1).unwrap();

    let replacement = fixture.path().join("replacement.jsonl");
    std::fs::write(
      &replacement,
      concat!(
        "{\"type\":\"session\",\"id\":\"new-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
        "{\"type\":\"message\",\"id\":\"1\",\"message\":{\"role\":\"user\",\"content\":\"old\"}}\n",
        "{\"type\":\"message\",\"id\":\"2\",\"message\":{\"role\":\"user\",\"content\":\"recent\"}}\n"
      ),
    )
    .unwrap();
    std::fs::rename(replacement, &path).unwrap();

    let update = tailer.start(false).unwrap();
    assert_eq!(update.events.len(), 1);
    let AgentEvent::Message(message) = &update.events[0].event else {
      panic!("expected message");
    };
    assert_eq!(message.text, "recent");
    assert_eq!(message.session_id.as_deref(), Some("new-session"));
  }

  #[test]
  fn buffers_partial_lines_until_newline() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("session_test.jsonl");
    std::fs::write(
      &path,
      "{\"type\":\"session\",\"id\":\"pi-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
    )
    .unwrap();
    let (mut tailer, _) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      false,
      3,
    )
    .unwrap();

    append(
      &path,
      "{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}",
    );
    assert!(tailer.scan().unwrap().events.is_empty());
    append(&path, "\n");
    assert_eq!(tailer.scan().unwrap().events.len(), 1);
  }

  #[test]
  fn preserves_a_partial_line_present_at_startup() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("session_test.jsonl");
    std::fs::write(
      &path,
      concat!(
        "{\"type\":\"session\",\"id\":\"pi-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
        "{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}"
      ),
    )
    .unwrap();
    let (mut tailer, _) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      false,
      3,
    )
    .unwrap();

    append(&path, "\n");
    let update = tailer.scan().unwrap();
    assert_eq!(update.events.len(), 1);
    let AgentEvent::Message(message) = &update.events[0].event else {
      panic!("expected message");
    };
    assert_eq!(message.text, "hello");
  }

  #[test]
  fn publishes_new_files_from_the_beginning() {
    let fixture = TempDir::new().unwrap();
    let (mut tailer, _) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      false,
      3,
    )
    .unwrap();
    std::fs::write(
      fixture.path().join("session_new.jsonl"),
      concat!(
        "{\"type\":\"session\",\"id\":\"new-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
        "{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n"
      ),
    )
    .unwrap();

    let update = tailer.scan().unwrap();
    assert_eq!(update.events.len(), 2);
    assert!(update.events.iter().all(|event| event.topic == "pi.new-session"));
  }

  #[test]
  fn backfills_new_files_from_the_third_most_recent_message() {
    let fixture = TempDir::new().unwrap();
    let (mut tailer, _) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      false,
      3,
    )
    .unwrap();
    std::fs::write(
      fixture.path().join("session_new.jsonl"),
      concat!(
        "{\"type\":\"session\",\"id\":\"new-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
        "{\"type\":\"message\",\"id\":\"1\",\"message\":{\"role\":\"user\",\"content\":\"one\"}}\n",
        "{\"type\":\"error\",\"message\":\"before window\"}\n",
        "{\"type\":\"message\",\"id\":\"2\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"two\"}]}}\n",
        "{\"type\":\"message\",\"id\":\"3\",\"message\":{\"role\":\"user\",\"content\":\"three\"}}\n",
        "{\"type\":\"error\",\"message\":\"inside window\"}\n",
        "{\"type\":\"message\",\"id\":\"4\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"four\"}]}}\n",
        "{\"type\":\"message\",\"id\":\"5\",\"message\":{\"role\":\"user\",\"content\":\"five\"}}\n"
      ),
    )
    .unwrap();

    let update = tailer.scan().unwrap();
    assert_eq!(update.events.len(), 4);
    let texts = update
      .events
      .iter()
      .filter_map(|event| match &event.event {
        AgentEvent::Message(message) => Some(message.text.as_str()),
        _ => None,
      })
      .collect::<Vec<_>>();
    assert_eq!(texts, ["three", "four", "five"]);
    assert!(
      update
        .events
        .iter()
        .any(|event| matches!(event.event, AgentEvent::Error(_)))
    );
  }

  #[test]
  fn seeds_codex_context_before_following() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("rollout-session-fixture.jsonl");
    std::fs::write(
      &path,
      "{\"timestamp\":\"2026-06-04T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-session\",\"timestamp\":\"2026-06-04T00:00:00Z\",\"cwd\":\"/tmp/project\",\"model_provider\":\"openai\"}}\n",
    )
    .unwrap();
    let (mut tailer, initial) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Codex, fixture.path().to_path_buf())],
      false,
      3,
    )
    .unwrap();
    assert!(initial.events.is_empty());

    append(
      &path,
      "{\"timestamp\":\"2026-06-04T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"hello\"}}\n",
    );
    let update = tailer.scan().unwrap();
    assert_eq!(update.events.len(), 1);
    assert_eq!(update.events[0].topic, "codex.codex-session");
    let AgentEvent::Message(message) = &update.events[0].event else {
      panic!("expected message");
    };
    assert_eq!(message.text, "hello");
    assert_eq!(message.session_id.as_deref(), Some("codex-session"));
  }

  #[test]
  fn follows_atomically_replaced_session_files() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("session_test.jsonl");
    std::fs::write(
      &path,
      "{\"type\":\"session\",\"id\":\"old-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
    )
    .unwrap();
    let (mut tailer, _) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      false,
      3,
    )
    .unwrap();

    let replacement = fixture.path().join("replacement");
    std::fs::write(
      &replacement,
      concat!(
        "{\"type\":\"session\",\"id\":\"new-session\",\"timestamp\":\"2026-01-02\",\"cwd\":\"/tmp\"}\n",
        "{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n"
      ),
    )
    .unwrap();
    std::fs::rename(replacement, &path).unwrap();

    let update = tailer.scan().unwrap();
    assert_eq!(update.events.len(), 2);
    assert!(update.events.iter().all(|event| event.topic == "pi.new-session"));
  }

  fn append(path: &std::path::Path, value: &str) {
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(value.as_bytes()).unwrap();
    file.flush().unwrap();
  }
}
