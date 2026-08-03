use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::event::CodexLine;
use crate::normalize::{CodexHistoryBoundary, CodexNormalizer};
use tokn_session_core::{LoadedSession, SessionHistoryStatus, SessionRef};

pub struct CodexSessionSource {
  session_dir: Option<PathBuf>,
}

impl CodexSessionSource {
  pub fn new(session_dir: Option<PathBuf>) -> Self {
    Self { session_dir }
  }

  pub fn list_sessions(&self) -> Result<Vec<SessionRef>, String> {
    self.list_session_refs(inspect_session)
  }

  pub fn list_session_relations(&self) -> Result<Vec<SessionRef>, String> {
    self.list_session_refs(inspect_session_header)
  }

  fn list_session_refs(&self, inspect: fn(&Path) -> Result<SessionRef, String>) -> Result<Vec<SessionRef>, String> {
    let mut paths = Vec::new();
    for root in self.roots()? {
      collect_jsonl_files(&root, &mut paths)?;
    }

    let mut refs = Vec::new();
    for path in paths {
      if let Ok(reference) = inspect(&path) {
        refs.push(reference);
      }
    }
    refs.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| b.path.cmp(&a.path)));
    Ok(refs)
  }

  pub fn load_session(&self, id_or_path: &str) -> Result<LoadedSession, String> {
    let path = self.resolve_session(id_or_path)?;
    self.load_session_path(&path)
  }

  pub fn load_session_path(&self, path: &Path) -> Result<LoadedSession, String> {
    let reference = inspect_session(&path)?;
    let file = File::open(&path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let reader = BufReader::new(file);
    let mut normalizer = CodexNormalizer::new_historical();
    let mut events = Vec::new();

    for (index, line) in reader.lines().enumerate() {
      let line = line.map_err(|err| format!("failed to read {}: {err}", path.display()))?;
      if line.trim().is_empty() {
        continue;
      }
      let event: CodexLine = serde_json::from_str(&line)
        .map_err(|err| format!("invalid codex jsonl at {}:{}: {err}", path.display(), index + 1))?;
      events.extend(normalizer.normalize(event));
    }

    Ok(LoadedSession {
      reference,
      events,
      history_status: normalizer.history_status(),
    })
  }

  fn resolve_session(&self, id_or_path: &str) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(id_or_path);
    if candidate.exists() {
      return Ok(candidate);
    }

    let matches: Vec<_> = self
      .list_sessions()?
      .into_iter()
      .filter(|session| session.id == id_or_path || session.id.starts_with(id_or_path))
      .collect();

    match matches.as_slice() {
      [session] => Ok(session.path.clone()),
      [] => Err(format!("no codex session found for `{id_or_path}`")),
      _ => Err(format!("multiple codex sessions match `{id_or_path}`")),
    }
  }

  fn roots(&self) -> Result<Vec<PathBuf>, String> {
    if let Some(root) = &self.session_dir {
      return Ok(vec![root.clone()]);
    }

    let home = std::env::var_os("HOME").ok_or_else(|| "HOME is not set".to_string())?;
    let codex_home = PathBuf::from(home).join(".codex");
    Ok(vec![codex_home.join("sessions"), codex_home.join("archived_sessions")])
  }
}

fn collect_jsonl_files(dir: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
  if !dir.exists() {
    return Ok(());
  }

  for entry in std::fs::read_dir(dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))? {
    let entry = entry.map_err(|err| format!("failed to read entry in {}: {err}", dir.display()))?;
    let path = entry.path();
    if path.is_dir() {
      collect_jsonl_files(&path, paths)?;
    } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
      paths.push(path);
    }
  }
  Ok(())
}

fn inspect_session(path: &Path) -> Result<SessionRef, String> {
  let mut reference = inspect_session_header(path)?;
  let file = File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
  let reader = BufReader::new(file);
  let mut message_count = 0;
  let mut history_boundary = CodexHistoryBoundary::new();

  for line in reader.lines() {
    let line = line.map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    if line.trim().is_empty() {
      continue;
    }
    let parsed_line: CodexLine =
      serde_json::from_str(&line).map_err(|err| format!("invalid codex jsonl at {}: {err}", path.display()))?;
    let value = parsed_line.native();
    let counts_as_message = counts_as_display_message(value);

    let accepted = history_boundary.accepts(parsed_line.item());
    if history_boundary.status() == SessionHistoryStatus::SubagentBodyUnavailable {
      message_count = 0;
    }
    if accepted && counts_as_message {
      message_count += 1;
    }
  }

  reference.message_count = message_count;
  Ok(reference)
}

fn inspect_session_header(path: &Path) -> Result<SessionRef, String> {
  let file = File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
  let reader = BufReader::new(file);
  let mut reference = SessionRef {
    id: session_id_from_path(path),
    parent_session_id: None,
    agent_path: None,
    agent_nickname: None,
    agent_role: None,
    path: path.to_path_buf(),
    cwd: None,
    timestamp: None,
    message_count: 0,
  };

  for line in reader.lines() {
    let line = line.map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    if line.trim().is_empty() {
      continue;
    }
    let parsed_line: CodexLine =
      serde_json::from_str(&line).map_err(|err| format!("invalid codex jsonl at {}: {err}", path.display()))?;
    let value = parsed_line.native();
    if value.get("type").and_then(|value| value.as_str()) != Some("session_meta") {
      continue;
    }
    let Some(payload) = value.get("payload") else {
      break;
    };

    if let Some(value) = payload.get("id").and_then(|value| value.as_str()) {
      reference.id = value.to_string();
    }
    // A user fork is a new root session. Only Codex's explicit parent-thread
    // relationship identifies a session that belongs in the subagent tree.
    reference.parent_session_id = string_field(payload, "parent_thread_id");
    let thread_spawn = payload
      .get("source")
      .and_then(|source| source.get("subagent"))
      .and_then(|subagent| subagent.get("thread_spawn"));
    reference.agent_path = string_field(payload, "agent_path")
      .or_else(|| thread_spawn.and_then(|thread_spawn| string_field(thread_spawn, "agent_path")));
    reference.agent_nickname = string_field(payload, "agent_nickname")
      .or_else(|| thread_spawn.and_then(|thread_spawn| string_field(thread_spawn, "agent_nickname")));
    reference.agent_role = first_string_field(payload, &["agent_role", "agent_type"]).or_else(|| {
      thread_spawn.and_then(|thread_spawn| first_string_field(thread_spawn, &["agent_role", "agent_type"]))
    });
    reference.cwd = string_field(payload, "cwd");
    reference.timestamp = string_field(payload, "timestamp").or_else(|| {
      value
        .get("timestamp")
        .and_then(|value| value.as_str())
        .map(str::to_string)
    });
    break;
  }

  Ok(reference)
}

fn counts_as_display_message(value: &serde_json::Value) -> bool {
  match value.get("type").and_then(|value| value.as_str()) {
    Some("response_item") => value.get("payload").is_some_and(|payload| {
      payload.get("type").and_then(|value| value.as_str()) == Some("message")
        && payload.get("role").and_then(|value| value.as_str()) == Some("assistant")
    }),
    Some("event_msg") => {
      value
        .get("payload")
        .and_then(|payload| payload.get("type"))
        .and_then(|value| value.as_str())
        == Some("user_message")
    }
    _ => false,
  }
}

fn string_field(value: &serde_json::Value, field: &str) -> Option<String> {
  value.get(field).and_then(|value| value.as_str()).map(str::to_string)
}

fn first_string_field(value: &serde_json::Value, fields: &[&str]) -> Option<String> {
  fields.iter().find_map(|field| string_field(value, field))
}

fn session_id_from_path(path: &Path) -> String {
  path
    .file_stem()
    .and_then(|value| value.to_str())
    .and_then(|stem| stem.rsplit_once('-').map(|(_, id)| id.to_string()))
    .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
  use super::*;
  use tokn_session_core::AgentEvent;

  #[test]
  fn first_session_meta_owns_subagent_identity() {
    let reference = inspect_session(&fixtures_dir().join("tree_child.jsonl")).expect("fixture should be inspectable");

    assert_eq!(reference.id, "tree-child");
    assert_eq!(reference.parent_session_id.as_deref(), Some("tree-root"));
    assert_eq!(reference.agent_path.as_deref(), Some("/root/researcher"));
    assert_eq!(reference.agent_nickname.as_deref(), Some("Hubble"));
    assert_eq!(reference.agent_role.as_deref(), Some("explorer"));
    assert_eq!(reference.timestamp.as_deref(), Some("2026-07-29T00:01:00Z"));
    assert_eq!(reference.message_count, 1);
  }

  #[test]
  fn treats_user_forks_as_root_sessions() {
    let reference = inspect_session(&fixtures_dir().join("forked_session.jsonl")).expect("fork should be inspectable");

    assert_eq!(reference.id, "forked-session");
    assert_eq!(reference.parent_session_id, None);
    assert_eq!(reference.agent_path, None);
    assert_eq!(reference.message_count, 2);
  }

  #[test]
  fn relation_scan_reads_identity_without_counting_the_body() {
    let source = CodexSessionSource::new(Some(fixtures_dir()));
    let references = source.list_session_relations().expect("fixture relations should load");
    let reference = references
      .iter()
      .find(|reference| reference.id == "tree-child")
      .expect("tree child should be discovered");

    assert_eq!(reference.parent_session_id.as_deref(), Some("tree-root"));
    assert_eq!(reference.agent_path.as_deref(), Some("/root/researcher"));
    assert_eq!(reference.message_count, 0);
  }

  #[test]
  fn loads_only_owned_subagent_history_after_the_trigger_boundary() {
    let source = CodexSessionSource::new(Some(fixtures_dir()));
    let loaded = source
      .load_session_path(&fixtures_dir().join("tree_child.jsonl"))
      .expect("child session should load from its selected path");

    assert_eq!(loaded.reference.id, "tree-child");
    assert_eq!(loaded.reference.parent_session_id.as_deref(), Some("tree-root"));
    assert_eq!(loaded.reference.message_count, 1);
    assert_eq!(loaded.history_status, SessionHistoryStatus::FilteredSubagent);
    let messages: Vec<_> = loaded
      .events
      .iter()
      .filter_map(|event| match event {
        AgentEvent::Message(event) => Some(event.text.as_str()),
        _ => None,
      })
      .collect();
    assert_eq!(messages, vec!["child result"]);
    assert!(loaded.events.iter().any(
      |event| matches!(event, AgentEvent::AgentActivity(event) if event.target_agent_path.as_deref() == Some("/root/researcher"))
    ));
  }

  #[test]
  fn reports_subagent_body_unavailable_without_a_trigger_boundary() {
    let source = CodexSessionSource::new(Some(fixtures_dir()));
    let loaded = source
      .load_session_path(&fixtures_dir().join("tree_child_no_boundary.jsonl"))
      .expect("incomplete child session should still load");

    assert_eq!(loaded.reference.message_count, 0);
    assert_eq!(loaded.history_status, SessionHistoryStatus::SubagentBodyUnavailable);
    assert_eq!(loaded.events.len(), 1);
    assert!(matches!(
      &loaded.events[0],
      AgentEvent::SessionStarted(event) if event.session_id == "tree-child-no-boundary"
    ));
  }

  #[test]
  fn keeps_non_thread_spawn_subagent_history_without_a_boundary() {
    let source = CodexSessionSource::new(Some(fixtures_dir()));
    let loaded = source
      .load_session_path(&fixtures_dir().join("tree_guardian.jsonl"))
      .expect("guardian session should load");

    assert_eq!(loaded.reference.parent_session_id.as_deref(), Some("guardian-parent"));
    assert_eq!(loaded.reference.message_count, 2);
    assert_eq!(loaded.history_status, SessionHistoryStatus::Complete);
    let messages: Vec<_> = loaded
      .events
      .iter()
      .filter_map(|event| match event {
        AgentEvent::Message(event) => Some(event.text.as_str()),
        _ => None,
      })
      .collect();
    assert_eq!(messages, vec!["review this action", "allow"]);
  }

  fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
  }
}
