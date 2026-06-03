use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::event::CodexLine;
use crate::normalize::CodexNormalizer;
use tokn_agent_core::{LoadedSession, SessionRef};

pub struct CodexSessionSource {
  session_dir: Option<PathBuf>,
}

impl CodexSessionSource {
  pub fn new(session_dir: Option<PathBuf>) -> Self {
    Self { session_dir }
  }

  pub fn list_sessions(&self) -> Result<Vec<SessionRef>, String> {
    let mut paths = Vec::new();
    for root in self.roots()? {
      collect_jsonl_files(&root, &mut paths)?;
    }

    let mut refs = Vec::new();
    for path in paths {
      if let Ok(reference) = inspect_session(&path) {
        refs.push(reference);
      }
    }
    refs.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| b.path.cmp(&a.path)));
    Ok(refs)
  }

  pub fn load_session(&self, id_or_path: &str) -> Result<LoadedSession, String> {
    let path = self.resolve_session(id_or_path)?;
    let reference = inspect_session(&path)?;
    let file = File::open(&path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let reader = BufReader::new(file);
    let mut normalizer = CodexNormalizer::new();
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

    Ok(LoadedSession { reference, events })
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
  let file = File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
  let reader = BufReader::new(file);
  let mut id = session_id_from_path(path);
  let mut cwd = None;
  let mut timestamp = None;
  let mut message_count = 0;

  for line in reader.lines() {
    let line = line.map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    if line.trim().is_empty() {
      continue;
    }
    let value: serde_json::Value =
      serde_json::from_str(&line).map_err(|err| format!("invalid codex jsonl at {}: {err}", path.display()))?;
    let line_timestamp = value
      .get("timestamp")
      .and_then(|value| value.as_str())
      .map(str::to_string);
    match value.get("type").and_then(|value| value.as_str()) {
      Some("session_meta") => {
        if let Some(payload) = value.get("payload") {
          if let Some(value) = payload.get("id").and_then(|value| value.as_str()) {
            id = value.to_string();
          }
          cwd = payload.get("cwd").and_then(|value| value.as_str()).map(str::to_string);
          timestamp = payload
            .get("timestamp")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(line_timestamp);
        }
      }
      Some("response_item") => {
        if let Some(payload) = value.get("payload") {
          let is_message = payload.get("type").and_then(|value| value.as_str()) == Some("message");
          let is_display_role = payload
            .get("role")
            .and_then(|value| value.as_str())
            .is_some_and(|role| role == "assistant");
          if is_message && is_display_role {
            message_count += 1;
          }
        }
      }
      Some("event_msg") => match value
        .get("payload")
        .and_then(|payload| payload.get("type"))
        .and_then(|value| value.as_str())
      {
        Some("user_message") => message_count += 1,
        _ => {}
      },
      _ => {}
    }
  }

  Ok(SessionRef {
    id,
    path: path.to_path_buf(),
    cwd,
    timestamp,
    message_count,
  })
}

fn session_id_from_path(path: &Path) -> String {
  path
    .file_stem()
    .and_then(|value| value.to_str())
    .and_then(|stem| stem.rsplit_once('-').map(|(_, id)| id.to_string()))
    .unwrap_or_else(|| path.display().to_string())
}
