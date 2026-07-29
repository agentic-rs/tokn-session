use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::event::PiSessionLine;
use crate::normalize::PiNormalizer;
use tokn_session_core::{LoadedSession, SessionHistoryStatus, SessionRef};

pub struct PiSessionSource {
  session_dir: Option<PathBuf>,
}

impl PiSessionSource {
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
    let mut sessions = Vec::new();
    let root = self.root()?;
    collect_jsonl_files(&root, &mut sessions)?;

    let mut refs = Vec::new();
    for path in sessions {
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
    let mut normalizer = PiNormalizer::new();
    let mut events = Vec::new();

    for (index, line) in reader.lines().enumerate() {
      let line = line.map_err(|err| format!("failed to read {}: {err}", path.display()))?;
      if line.trim().is_empty() {
        continue;
      }
      let event: PiSessionLine = serde_json::from_str(&line)
        .map_err(|err| format!("invalid pi jsonl at {}:{}: {err}", path.display(), index + 1))?;
      events.extend(normalizer.normalize(event));
    }

    Ok(LoadedSession {
      reference,
      events,
      history_status: SessionHistoryStatus::Complete,
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
      [] => Err(format!("no pi session found for `{id_or_path}`")),
      _ => Err(format!("multiple pi sessions match `{id_or_path}`")),
    }
  }

  fn root(&self) -> Result<PathBuf, String> {
    if let Some(root) = &self.session_dir {
      return Ok(root.clone());
    }

    let home = std::env::var_os("HOME").ok_or_else(|| "HOME is not set".to_string())?;
    Ok(PathBuf::from(home).join(".pi").join("agent").join("sessions"))
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

  for line in reader.lines() {
    let line = line.map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    if line.trim().is_empty() {
      continue;
    }
    let value: serde_json::Value =
      serde_json::from_str(&line).map_err(|err| format!("invalid pi jsonl at {}: {err}", path.display()))?;
    if value.get("type").and_then(|value| value.as_str()) == Some("message") {
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
    let value: serde_json::Value =
      serde_json::from_str(&line).map_err(|err| format!("invalid pi jsonl at {}: {err}", path.display()))?;
    if value.get("type").and_then(|value| value.as_str()) != Some("session") {
      continue;
    }

    if let Some(value) = value.get("id").and_then(|value| value.as_str()) {
      reference.id = value.to_string();
    }
    reference.parent_session_id = value
      .get("parentSession")
      .and_then(|value| value.as_str())
      .map(|parent| resolve_parent_session_id(path, parent));
    reference.cwd = value.get("cwd").and_then(|value| value.as_str()).map(str::to_string);
    reference.timestamp = value
      .get("timestamp")
      .and_then(|value| value.as_str())
      .map(str::to_string);
    break;
  }

  Ok(reference)
}

fn resolve_parent_session_id(session_path: &Path, parent: &str) -> String {
  let parent_path = PathBuf::from(parent);
  let parent_path = if parent_path.is_absolute() {
    parent_path
  } else {
    session_path
      .parent()
      .map(|directory| directory.join(&parent_path))
      .unwrap_or(parent_path)
  };

  let parent_id = File::open(&parent_path).ok().and_then(|file| {
    BufReader::new(file).lines().map_while(Result::ok).find_map(|line| {
      let value: serde_json::Value = serde_json::from_str(&line).ok()?;
      if value.get("type").and_then(|value| value.as_str()) != Some("session") {
        return None;
      }
      value.get("id").and_then(|value| value.as_str()).map(str::to_string)
    })
  });

  parent_id.unwrap_or_else(|| session_id_from_path(&parent_path))
}

fn session_id_from_path(path: &Path) -> String {
  path
    .file_stem()
    .and_then(|value| value.to_str())
    .and_then(|stem| stem.rsplit_once('_').map(|(_, id)| id.to_string()))
    .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn resolves_parent_session_path_to_parent_id() {
    let reference = inspect_session(&fixtures_dir().join("tree_child.jsonl")).expect("fixture should be inspectable");

    assert_eq!(reference.id, "pi-tree-child");
    assert_eq!(reference.parent_session_id.as_deref(), Some("pi-tree-root"));
  }

  #[test]
  fn relation_scan_reads_parent_without_counting_the_body() {
    let source = PiSessionSource::new(Some(fixtures_dir()));
    let references = source.list_session_relations().expect("fixture relations should load");
    let reference = references
      .iter()
      .find(|reference| reference.id == "pi-tree-child")
      .expect("tree child should be discovered");

    assert_eq!(reference.parent_session_id.as_deref(), Some("pi-tree-root"));
    assert_eq!(reference.message_count, 0);
  }

  fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
  }
}
