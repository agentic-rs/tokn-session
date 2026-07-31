use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use serde::Serialize;
use serde_json::Value;
use tokn_session_codex::{event::CodexLine, normalize::CodexNormalizer};
use tokn_session_core::{AgentEvent, LoadedSession, Provider};
use tokn_session_opencode::OpenCodeSessionSource;
use tokn_session_pi::{event::PiSessionLine, normalize::PiNormalizer};

use crate::context::session_id_from_path;
use crate::project::ProjectCatalog;
use crate::{NewFileReplay, SessionContext};

type SharedProjectCatalog = Arc<RwLock<ProjectCatalog>>;

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

#[derive(Debug, Serialize)]
pub struct RelayEvent {
  pub path: PathBuf,
  pub topic: String,
  pub session: SessionContext,
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
  opencode: HashMap<PathBuf, OpenCodeState>,
  new_file_replay: NewFileReplay,
  project_catalog: SharedProjectCatalog,
  project_catalog_source: Option<ProjectCatalogSource>,
  project_catalog_warning: Option<String>,
}

impl SessionTailer {
  pub fn initialize(roots: Vec<ProviderRoot>, new_file_replay: NewFileReplay) -> Result<(Self, TailUpdate), String> {
    let mut tailer = Self::prepare(roots, new_file_replay)?;
    let update = tailer.start()?;
    Ok((tailer, update))
  }

  pub(crate) fn prepare(roots: Vec<ProviderRoot>, new_file_replay: NewFileReplay) -> Result<Self, String> {
    let (project_catalog, project_catalog_source, project_catalog_warning) = load_project_catalog(&roots);
    let project_catalog = Arc::new(RwLock::new(project_catalog));
    let mut tailer = Self {
      roots,
      files: HashMap::new(),
      opencode: HashMap::new(),
      new_file_replay,
      project_catalog,
      project_catalog_source,
      project_catalog_warning,
    };
    for root in &tailer.roots {
      if matches!(root.provider, Provider::OpenCode) {
        tailer
          .opencode
          .insert(root.path.clone(), OpenCodeState::new(root.path.clone()));
      }
    }
    let paths = tailer.discover_paths()?;
    for (path, provider) in paths {
      tailer.files.insert(
        path.clone(),
        FileState::open(path, provider, Arc::clone(&tailer.project_catalog))?,
      );
    }
    Ok(tailer)
  }

  pub(crate) fn start(&mut self) -> Result<TailUpdate, String> {
    let mut update = TailUpdate::default();
    if let Some(warning) = self.project_catalog_warning.take() {
      update.warnings.push(warning);
    }
    let new_file_replay = self.new_file_replay;
    for state in self.files.values_mut() {
      let mode = if state.matches_initial_snapshot()? {
        InitialRead::Follow
      } else {
        InitialRead::Replay(new_file_replay)
      };
      let initial = read_initial(state, mode)?;
      update.append(initial);
    }
    for state in self.opencode.values_mut() {
      update.append(state.scan(false, self.new_file_replay)?);
    }
    Ok(update)
  }

  pub fn scan(&mut self) -> Result<TailUpdate, String> {
    let discovered = self.discover_paths()?;
    let mut update = TailUpdate::default();
    self.refresh_project_catalog(&mut update);

    for (path, provider) in discovered {
      if !self.files.contains_key(&path) {
        self.add_file(path, provider, InitialRead::Replay(self.new_file_replay), &mut update)?;
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
        apply_replay_policy(&mut appended.events, self.new_file_replay);
      }
      update.append(appended);
    }

    for state in self.opencode.values_mut() {
      update.append(state.scan(true, self.new_file_replay)?);
    }

    Ok(update)
  }

  pub fn scan_paths(&mut self, changed_paths: HashSet<PathBuf>) -> Result<TailUpdate, String> {
    let mut update = TailUpdate::default();
    self.refresh_project_catalog(&mut update);
    let mut candidates = HashMap::new();
    let mut changed_directories = Vec::new();
    let mut changed_opencode = HashSet::new();

    for path in changed_paths {
      let Some(provider) = self.provider_for_path(&path) else {
        continue;
      };
      if matches!(provider, Provider::OpenCode) {
        if let Some(root) = self.open_code_root_for_path(&path) {
          changed_opencode.insert(root);
        }
        continue;
      }
      if path.is_dir() {
        changed_directories.push(path.clone());
        let mut seen = HashSet::new();
        let mut discovered = Vec::new();
        collect_jsonl_files(&path, provider, &mut seen, &mut discovered)?;
        candidates.extend(discovered);
      } else if is_jsonl(&path) {
        candidates.insert(path, provider);
      } else if !path.exists() {
        changed_directories.push(path);
      }
    }

    if !changed_directories.is_empty() {
      self.files.retain(|path, _| {
        !changed_directories
          .iter()
          .any(|directory| path.starts_with(directory) && !path.exists())
      });
    }

    for (path, provider) in candidates {
      self.scan_file(path, provider, &mut update)?;
    }
    for path in changed_opencode {
      if let Some(state) = self.opencode.get_mut(&path) {
        update.append(state.scan(true, self.new_file_replay)?);
      }
    }
    Ok(update)
  }

  pub fn roots(&self) -> &[ProviderRoot] {
    &self.roots
  }

  fn scan_file(&mut self, path: PathBuf, provider: Provider, update: &mut TailUpdate) -> Result<(), String> {
    if !path.exists() {
      self.files.remove(&path);
      return Ok(());
    }
    let Some(state) = self.files.get_mut(&path) else {
      return self.add_file(path, provider, InitialRead::Replay(self.new_file_replay), update);
    };
    let (mut appended, restarted) = state.read_appended(true)?;
    if restarted {
      apply_replay_policy(&mut appended.events, self.new_file_replay);
    }
    update.append(appended);
    Ok(())
  }

  fn add_file(
    &mut self,
    path: PathBuf,
    provider: Provider,
    mode: InitialRead,
    update: &mut TailUpdate,
  ) -> Result<(), String> {
    let mut state = FileState::open(path.clone(), provider, Arc::clone(&self.project_catalog))?;
    let initial = read_initial(&mut state, mode)?;
    update.append(initial);
    self.files.insert(path, state);
    Ok(())
  }

  fn discover_paths(&self) -> Result<Vec<(PathBuf, Provider)>, String> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for root in &self.roots {
      if !matches!(root.provider, Provider::OpenCode) {
        collect_jsonl_files(&root.path, root.provider, &mut seen, &mut paths)?;
      }
    }
    Ok(paths)
  }

  fn provider_for_path(&self, path: &Path) -> Option<Provider> {
    self
      .roots
      .iter()
      .filter(|root| path.starts_with(&root.path))
      .max_by_key(|root| root.path.components().count())
      .map(|root| root.provider)
  }

  fn open_code_root_for_path(&self, path: &Path) -> Option<PathBuf> {
    self
      .roots
      .iter()
      .filter(|root| matches!(root.provider, Provider::OpenCode) && path.starts_with(&root.path))
      .max_by_key(|root| root.path.components().count())
      .map(|root| root.path.clone())
  }

  fn refresh_project_catalog(&mut self, update: &mut TailUpdate) {
    let Some(source) = &mut self.project_catalog_source else {
      return;
    };
    let Some(result) = source.load_if_changed() else {
      return;
    };
    match result {
      Ok(catalog) => {
        *self
          .project_catalog
          .write()
          .unwrap_or_else(|poisoned| poisoned.into_inner()) = catalog;
      }
      Err(error) => update.warnings.push(error),
    }
  }
}

fn read_initial(state: &mut FileState, mode: InitialRead) -> Result<TailUpdate, String> {
  match mode {
    InitialRead::Follow => state.seed_at_eof(),
    InitialRead::Replay(NewFileReplay::All) => Ok(state.read_appended(true)?.0),
    InitialRead::Replay(NewFileReplay::Messages(message_count)) => {
      let mut update = state.read_appended(true)?.0;
      retain_message_history(&mut update.events, message_count);
      Ok(update)
    }
  }
}

#[derive(Clone, Copy)]
enum InitialRead {
  Follow,
  Replay(NewFileReplay),
}

impl TailUpdate {
  fn append(&mut self, mut other: Self) {
    self.events.append(&mut other.events);
    self.warnings.append(&mut other.warnings);
  }
}

struct OpenCodeState {
  root_path: PathBuf,
  source: OpenCodeSessionSource,
  sessions: HashMap<String, OpenCodeSessionState>,
}

struct OpenCodeSessionState {
  fingerprints: Vec<String>,
}

impl OpenCodeState {
  fn new(root_path: PathBuf) -> Self {
    Self {
      source: OpenCodeSessionSource::new(Some(root_path.clone())),
      root_path,
      sessions: HashMap::new(),
    }
  }

  fn scan(&mut self, publish_new_sessions: bool, replay: NewFileReplay) -> Result<TailUpdate, String> {
    if !self.database_exists() {
      return Ok(TailUpdate::default());
    }

    let references = self.source.list_sessions()?;
    let mut seen = HashSet::new();
    let mut update = TailUpdate::default();
    for reference in references {
      let session_id = reference.id.clone();
      seen.insert(session_id.clone());
      let loaded = self.source.load_session_exact(&session_id)?;
      let fingerprints = loaded.events.iter().map(event_fingerprint).collect::<Vec<_>>();
      let context = SessionContext::from_session_ref(&loaded.reference);
      let events = relay_events_from_loaded(loaded, &context, &fingerprints, self.sessions.get(&session_id));

      match self.sessions.get(&session_id) {
        None if publish_new_sessions => {
          let mut events = events;
          apply_replay_policy(&mut events, replay);
          update.events.extend(events);
        }
        Some(_) if publish_new_sessions => {
          update.events.extend(events);
        }
        _ => {}
      }

      self.sessions.insert(session_id, OpenCodeSessionState { fingerprints });
    }
    self.sessions.retain(|session_id, _| seen.contains(session_id));
    Ok(update)
  }

  fn database_exists(&self) -> bool {
    if self.root_path.is_dir() {
      self.root_path.join("opencode.db").is_file()
    } else {
      self.root_path.is_file()
    }
  }
}

fn relay_events_from_loaded(
  loaded: LoadedSession,
  context: &SessionContext,
  fingerprints: &[String],
  previous: Option<&OpenCodeSessionState>,
) -> Vec<RelayEvent> {
  let path = loaded.reference.path.clone();
  loaded
    .events
    .into_iter()
    .enumerate()
    .filter_map(|(index, event)| {
      let changed = previous
        .and_then(|previous| previous.fingerprints.get(index))
        .is_none_or(|fingerprint| fingerprint != &fingerprints[index]);
      changed.then(|| RelayEvent {
        topic: event_topic(Provider::OpenCode, &path, &event),
        path: path.clone(),
        session: context.clone(),
        event,
      })
    })
    .collect()
}

fn event_fingerprint(event: &AgentEvent) -> String {
  serde_json::to_string(event).unwrap_or_else(|_| format!("{event:?}"))
}

struct FileState {
  path: PathBuf,
  provider: Provider,
  identity: FileIdentity,
  initial_length: u64,
  offset: u64,
  pending: Vec<u8>,
  normalizer: SessionNormalizer,
  context: SessionContext,
  project_catalog: SharedProjectCatalog,
}

impl FileState {
  fn open(path: PathBuf, provider: Provider, project_catalog: SharedProjectCatalog) -> Result<Self, String> {
    let metadata = std::fs::metadata(&path).map_err(|err| format!("failed to inspect {}: {err}", path.display()))?;
    let context = SessionContext::from_path(provider, &path);
    Ok(Self {
      path,
      provider,
      identity: file_identity(&metadata),
      initial_length: metadata.len(),
      offset: 0,
      pending: Vec::new(),
      normalizer: SessionNormalizer::new(provider),
      context,
      project_catalog,
    })
  }

  fn seed_at_eof(&mut self) -> Result<TailUpdate, String> {
    const MAX_SESSION_HEADER_LINES: usize = 64;

    let file = File::open(&self.path).map_err(|err| format!("failed to open {}: {err}", self.path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut update = TailUpdate::default();
    let project_catalog = self
      .project_catalog
      .read()
      .unwrap_or_else(|poisoned| poisoned.into_inner());
    for _ in 0..MAX_SESSION_HEADER_LINES {
      line.clear();
      let bytes = reader
        .read_line(&mut line)
        .map_err(|err| format!("failed to read {}: {err}", self.path.display()))?;
      if bytes == 0 || !line.ends_with('\n') {
        break;
      }
      match self
        .normalizer
        .normalize_line(line.trim_end_matches(['\r', '\n']), &mut self.context, &project_catalog)
      {
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
      self.context = SessionContext::from_path(self.provider, &self.path);
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
    let project_catalog = self
      .project_catalog
      .read()
      .unwrap_or_else(|poisoned| poisoned.into_inner());
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

      match self
        .normalizer
        .normalize_line(line, &mut self.context, &project_catalog)
      {
        Ok(events) if should_publish => {
          update.events.extend(events.into_iter().map(|event| RelayEvent {
            topic: event_topic(self.provider, &self.path, &event),
            path: self.path.clone(),
            session: self.context.clone(),
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

fn apply_replay_policy(events: &mut Vec<RelayEvent>, replay: NewFileReplay) {
  if let NewFileReplay::Messages(message_count) = replay {
    retain_message_history(events, message_count);
  }
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

  fn normalize_line(
    &mut self,
    line: &str,
    context: &mut SessionContext,
    project_catalog: &ProjectCatalog,
  ) -> Result<Vec<AgentEvent>, String> {
    let value: Value = serde_json::from_str(line).map_err(|err| format!("invalid session JSONL: {err}"))?;
    context.update(&value);
    if matches!(context.provider, Provider::Codex) {
      context.resolve_project_name(project_catalog);
    }

    match self {
      Self::Codex(normalizer) => {
        let event: CodexLine = serde_json::from_value(value).map_err(|err| format!("invalid codex JSONL: {err}"))?;
        Ok(normalizer.normalize(event))
      }
      Self::Pi(normalizer) => {
        let event: PiSessionLine = serde_json::from_value(value).map_err(|err| format!("invalid pi JSONL: {err}"))?;
        Ok(normalizer.normalize(event))
      }
    }
  }
}

fn load_project_catalog(roots: &[ProviderRoot]) -> (ProjectCatalog, Option<ProjectCatalogSource>, Option<String>) {
  let Some(path) = roots
    .iter()
    .find(|root| matches!(root.provider, Provider::Codex))
    .and_then(|root| root.path.parent())
    .map(|codex_home| codex_home.join(".codex-global-state.json"))
  else {
    return (ProjectCatalog::default(), None, None);
  };
  let mut source = ProjectCatalogSource {
    path,
    observed_version: None,
  };
  match source.load_if_changed() {
    Some(Ok(catalog)) => (catalog, Some(source), None),
    Some(Err(error)) => (ProjectCatalog::default(), Some(source), Some(error)),
    None => (ProjectCatalog::default(), Some(source), None),
  }
}

struct ProjectCatalogSource {
  path: PathBuf,
  observed_version: Option<CatalogFileVersion>,
}

impl ProjectCatalogSource {
  fn load_if_changed(&mut self) -> Option<Result<ProjectCatalog, String>> {
    let metadata = match std::fs::metadata(&self.path) {
      Ok(metadata) => metadata,
      Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
        return self.observed_version.take().map(|_| Ok(ProjectCatalog::default()));
      }
      Err(error) => {
        return Some(Err(format!(
          "failed to inspect Codex Desktop project catalog {}: {error}",
          self.path.display()
        )));
      }
    };
    let version = CatalogFileVersion {
      identity: file_identity(&metadata),
      length: metadata.len(),
      modified: metadata.modified().ok(),
    };
    if self.observed_version == Some(version) {
      return None;
    }
    self.observed_version = Some(version);
    Some(ProjectCatalog::load(&self.path))
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CatalogFileVersion {
  identity: FileIdentity,
  length: u64,
  modified: Option<SystemTime>,
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
    AgentEvent::SessionSettingsApplied(event) => event.session_id.as_deref(),
    AgentEvent::Message(event) => event.session_id.as_deref(),
    AgentEvent::Reasoning(event) => event.session_id.as_deref(),
    AgentEvent::GoalUpdated(event) => event.session_id.as_deref(),
    AgentEvent::AgentActivity(event) => event.session_id.as_deref(),
    AgentEvent::ToolCall(event) => event.session_id.as_deref(),
    AgentEvent::Error(event) => event.session_id.as_deref(),
    AgentEvent::Unknown(event) => event.session_id.as_deref(),
  }
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
  use std::collections::HashSet;
  use std::fs::OpenOptions;
  use std::io::Write;

  use tempfile::TempDir;
  use tokn_session_core::{AgentEvent, Provider};

  use super::{ProviderRoot, SessionTailer};
  use crate::NewFileReplay;

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
      NewFileReplay::Messages(3),
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
    assert_eq!(update.events[0].session.session_id, "pi-session");
    assert_eq!(update.events[0].session.started_at.as_deref(), Some("2026-01-01"));
    assert_eq!(
      update.events[0]
        .session
        .project
        .as_ref()
        .and_then(|project| project.name.as_deref()),
      Some("tmp")
    );
    let AgentEvent::Message(message) = &update.events[0].event else {
      panic!("expected message");
    };
    assert_eq!(message.text, "done");
    assert_eq!(message.session_id.as_deref(), Some("pi-session"));
  }

  #[test]
  fn replay_all_does_not_apply_to_files_present_at_startup() {
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
      NewFileReplay::All,
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
    let mut tailer = SessionTailer::prepare(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      NewFileReplay::Messages(3),
    )
    .unwrap();

    append(
      &path,
      "{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"user\",\"content\":\"not lost\"}}\n",
    );
    assert!(tailer.start().unwrap().events.is_empty());

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
    let mut tailer = SessionTailer::prepare(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      NewFileReplay::Messages(1),
    )
    .unwrap();

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

    let update = tailer.start().unwrap();
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
      NewFileReplay::Messages(3),
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
  fn scans_only_files_named_by_watcher_events() {
    let fixture = TempDir::new().unwrap();
    let first = fixture.path().join("session_first.jsonl");
    let second = fixture.path().join("session_second.jsonl");
    for (path, session_id) in [(&first, "first"), (&second, "second")] {
      std::fs::write(
        path,
        format!("{{\"type\":\"session\",\"id\":\"{session_id}\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}}\n"),
      )
      .unwrap();
    }
    let (mut tailer, _) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      NewFileReplay::Messages(3),
    )
    .unwrap();

    append(
      &first,
      "{\"type\":\"message\",\"id\":\"first\",\"message\":{\"role\":\"user\",\"content\":\"first\"}}\n",
    );
    append(
      &second,
      "{\"type\":\"message\",\"id\":\"second\",\"message\":{\"role\":\"user\",\"content\":\"second\"}}\n",
    );

    let first_update = tailer.scan_paths(HashSet::from([first])).unwrap();
    assert_eq!(first_update.events.len(), 1);
    assert_eq!(first_update.events[0].topic, "pi.first");

    let second_update = tailer.scan_paths(HashSet::from([second])).unwrap();
    assert_eq!(second_update.events.len(), 1);
    assert_eq!(second_update.events[0].topic, "pi.second");
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
      NewFileReplay::Messages(3),
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
  fn replays_all_events_for_new_files() {
    let fixture = TempDir::new().unwrap();
    let (mut tailer, _) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      NewFileReplay::All,
    )
    .unwrap();
    let path = fixture.path().join("session_new.jsonl");
    std::fs::write(
      &path,
      concat!(
        "{\"type\":\"session\",\"id\":\"new-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
        "{\"type\":\"message\",\"id\":\"old\",\"message\":{\"role\":\"user\",\"content\":\"old\"}}\n",
        "{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n"
      ),
    )
    .unwrap();

    let update = tailer.scan_paths(HashSet::from([path])).unwrap();
    assert_eq!(update.events.len(), 3);
    assert!(update.events.iter().all(|event| event.topic == "pi.new-session"));
  }

  #[test]
  fn backfills_new_files_from_the_third_most_recent_message() {
    let fixture = TempDir::new().unwrap();
    let (mut tailer, _) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())],
      NewFileReplay::Messages(3),
    )
    .unwrap();
    let path = fixture.path().join("session_new.jsonl");
    std::fs::write(
      &path,
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

    let update = tailer.scan_paths(HashSet::from([path])).unwrap();
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
      concat!(
        "{\"timestamp\":\"2026-06-04T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{",
        "\"id\":\"codex-session\",",
        "\"parent_thread_id\":\"parent-session\",",
        "\"agent_path\":\"/root/researcher\",",
        "\"agent_nickname\":\"Hubble\",",
        "\"agent_role\":\"explorer\",",
        "\"timestamp\":\"2026-06-04T00:00:00Z\",",
        "\"cwd\":\"/tmp/worktree\",",
        "\"model_provider\":\"openai\",",
        "\"git\":{",
        "\"commit_hash\":\"abcdef123456\",",
        "\"branch\":\"main\",",
        "\"repository_url\":\"https://github.com/agentic-rs/tokn-session.git\"",
        "}}}\n",
      ),
    )
    .unwrap();
    let (mut tailer, initial) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Codex, fixture.path().to_path_buf())],
      NewFileReplay::Messages(3),
    )
    .unwrap();
    assert!(initial.events.is_empty());

    append(
      &path,
      concat!(
        "{\"timestamp\":\"2026-06-04T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{",
        "\"type\":\"thread_settings_applied\",",
        "\"thread_settings\":{\"model\":\"gpt-5\",\"cwd\":\"/tmp/worktree/subdir\"}}}\n",
        "{\"timestamp\":\"2026-06-04T00:00:02Z\",\"type\":\"event_msg\",",
        "\"payload\":{\"type\":\"user_message\",\"message\":\"hello\"}}\n",
      ),
    );
    let update = tailer.scan_paths(HashSet::from([path])).unwrap();
    assert_eq!(update.events.len(), 2);
    assert_eq!(update.events[0].topic, "codex.codex-session");
    let context = &update.events[0].session;
    assert_eq!(context.session_id, "codex-session");
    assert_eq!(context.parent_session_id.as_deref(), Some("parent-session"));
    assert_eq!(context.agent_path.as_deref(), Some("/root/researcher"));
    assert_eq!(context.agent_nickname.as_deref(), Some("Hubble"));
    assert_eq!(context.agent_role.as_deref(), Some("explorer"));
    assert_eq!(context.cwd.as_deref(), Some("/tmp/worktree/subdir"));
    assert_eq!(context.started_at.as_deref(), Some("2026-06-04T00:00:00Z"));
    let relay_json = serde_json::to_value(&update.events[0]).unwrap();
    assert_eq!(relay_json["session"]["agent_path"], "/root/researcher");
    assert_eq!(relay_json["session"]["agent_nickname"], "Hubble");
    assert_eq!(relay_json["session"]["agent_role"], "explorer");
    let project = context.project.as_ref().unwrap();
    assert_eq!(
      project.id.as_deref(),
      Some("https://github.com/agentic-rs/tokn-session.git")
    );
    assert_eq!(project.name.as_deref(), Some("tokn-session"));
    assert_eq!(project.project_name, None);
    assert_eq!(project.folder.as_deref(), Some("/tmp/worktree"));
    assert_eq!(project.folder_name.as_deref(), Some("worktree"));
    assert_eq!(project.repository_name.as_deref(), Some("tokn-session"));
    assert_eq!(project.branch.as_deref(), Some("main"));
    assert_eq!(project.commit_hash.as_deref(), Some("abcdef123456"));
    let AgentEvent::SessionSettingsApplied(settings) = &update.events[0].event else {
      panic!("expected settings event");
    };
    assert_eq!(settings.cwd.as_deref(), Some("/tmp/worktree/subdir"));
    let AgentEvent::Message(message) = &update.events[1].event else {
      panic!("expected message");
    };
    assert_eq!(message.text, "hello");
    assert_eq!(message.session_id.as_deref(), Some("codex-session"));
  }

  #[test]
  fn adds_desktop_project_folder_and_repository_names_to_codex_events() {
    let fixture = TempDir::new().unwrap();
    let sessions = fixture.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    std::fs::write(
      fixture.path().join(".codex-global-state.json"),
      r#"{
        "local-projects": {
          "project-id": {
            "id": "project-id",
            "name": "llm-router_2",
            "rootPaths": ["/workspace/llm-router"]
          }
        },
        "thread-project-assignments": {
          "root-session": {
            "projectKind": "local",
            "projectId": "project-id",
            "cwd": "/workspace/llm-router"
          }
        }
      }"#,
    )
    .unwrap();
    let path = sessions.join("rollout-session-fixture.jsonl");
    std::fs::write(
      &path,
      concat!(
        "{\"timestamp\":\"2026-06-04T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{",
        "\"id\":\"child-session\",",
        "\"parent_thread_id\":\"root-session\",",
        "\"timestamp\":\"2026-06-04T00:00:00Z\",",
        "\"cwd\":\"/workspace/llm-router\",",
        "\"git\":{\"repository_url\":\"https://github.com/agentic-rs/tokn\"}",
        "}}\n",
      ),
    )
    .unwrap();

    let (mut tailer, initial) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Codex, sessions)],
      NewFileReplay::Messages(3),
    )
    .unwrap();
    assert!(initial.events.is_empty());
    assert!(initial.warnings.is_empty());

    append(
      &path,
      "{\"timestamp\":\"2026-06-04T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"hello\"}}\n",
    );
    let update = tailer.scan_paths(HashSet::from([path])).unwrap();
    assert_eq!(update.events.len(), 1);
    let project = update.events[0].session.project.as_ref().unwrap();
    assert_eq!(project.name.as_deref(), Some("tokn"));
    assert_eq!(project.project_name.as_deref(), Some("llm-router_2"));
    assert_eq!(project.folder.as_deref(), Some("/workspace/llm-router"));
    assert_eq!(project.folder_name.as_deref(), Some("llm-router"));
    assert_eq!(project.repository_name.as_deref(), Some("tokn"));
    assert_eq!(
      project.repository_url.as_deref(),
      Some("https://github.com/agentic-rs/tokn")
    );
  }

  #[test]
  fn reports_an_invalid_desktop_project_catalog_without_stopping() {
    let fixture = TempDir::new().unwrap();
    let sessions = fixture.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    std::fs::write(fixture.path().join(".codex-global-state.json"), "{not json").unwrap();

    let (_, initial) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Codex, sessions)],
      NewFileReplay::Messages(3),
    )
    .unwrap();

    assert!(initial.events.is_empty());
    assert_eq!(initial.warnings.len(), 1);
    assert!(initial.warnings[0].contains("Codex Desktop project catalog"));
  }

  #[test]
  fn reloads_desktop_project_catalog_after_startup() {
    let fixture = TempDir::new().unwrap();
    let sessions = fixture.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    let path = sessions.join("rollout-session-fixture.jsonl");
    std::fs::write(
      &path,
      concat!(
        "{\"timestamp\":\"2026-06-04T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{",
        "\"id\":\"child-session\",",
        "\"parent_thread_id\":\"root-session\",",
        "\"cwd\":\"/workspace/llm-router\"",
        "}}\n",
      ),
    )
    .unwrap();
    let (mut tailer, initial) = SessionTailer::initialize(
      vec![ProviderRoot::new(Provider::Codex, sessions)],
      NewFileReplay::Messages(3),
    )
    .unwrap();
    assert!(initial.warnings.is_empty());

    let state_path = fixture.path().join(".codex-global-state.json");
    std::fs::write(
      &state_path,
      r#"{
        "local-projects": {
          "project-id": {
            "name": "llm-router_2",
            "rootPaths": ["/workspace/llm-router"]
          }
        },
        "thread-project-assignments": {
          "root-session": {
            "projectKind": "local",
            "projectId": "project-id"
          }
        }
      }"#,
    )
    .unwrap();
    append(
      &path,
      "{\"timestamp\":\"2026-06-04T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"first\"}}\n",
    );
    let first = tailer.scan_paths(HashSet::from([path.clone()])).unwrap();
    assert_eq!(
      first.events[0]
        .session
        .project
        .as_ref()
        .and_then(|project| project.project_name.as_deref()),
      Some("llm-router_2")
    );

    let replacement = fixture.path().join("replacement-state.json");
    std::fs::write(
      &replacement,
      r#"{
        "local-projects": {
          "project-id": {
            "name": "llm-router-renamed",
            "rootPaths": ["/workspace/llm-router"]
          }
        },
        "thread-project-assignments": {
          "root-session": {
            "projectKind": "local",
            "projectId": "project-id"
          }
        }
      }"#,
    )
    .unwrap();
    std::fs::remove_file(&state_path).unwrap();
    std::fs::rename(replacement, &state_path).unwrap();
    append(
      &path,
      "{\"timestamp\":\"2026-06-04T00:00:02Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"second\"}}\n",
    );
    let second = tailer.scan_paths(HashSet::from([path.clone()])).unwrap();
    assert_eq!(
      second.events[0]
        .session
        .project
        .as_ref()
        .and_then(|project| project.project_name.as_deref()),
      Some("llm-router-renamed")
    );

    std::fs::remove_file(&state_path).unwrap();
    append(
      &path,
      "{\"timestamp\":\"2026-06-04T00:00:03Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"third\"}}\n",
    );
    let third = tailer.scan_paths(HashSet::from([path])).unwrap();
    assert_eq!(
      third.events[0]
        .session
        .project
        .as_ref()
        .and_then(|project| project.project_name.as_deref()),
      None
    );
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
      NewFileReplay::Messages(3),
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

    let update = tailer.scan_paths(HashSet::from([path])).unwrap();
    assert_eq!(update.events.len(), 2);
    assert!(update.events.iter().all(|event| event.topic == "pi.new-session"));
  }

  fn append(path: &std::path::Path, value: &str) {
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(value.as_bytes()).unwrap();
    file.flush().unwrap();
  }
}
