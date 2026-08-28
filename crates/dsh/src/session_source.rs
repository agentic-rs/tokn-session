use std::path::{Path, PathBuf};

use tokn_dsh_protocol::{DshSessionItem, DshSessionLine, SessionHeader};
use tokn_session_core::{LoadedSession, SessionHistoryStatus, SessionRef};

use crate::{normalize, storage};

pub struct DshSessionSource {
  session_dir: Option<PathBuf>,
}

impl DshSessionSource {
  pub fn new(session_dir: Option<PathBuf>) -> Self {
    Self { session_dir }
  }

  pub fn list_sessions(&self) -> Result<Vec<SessionRef>, String> {
    let mut references = Vec::new();
    for path in self.paths()? {
      // Unlike silently dropping unreadable sessions, an actionable path error
      // makes unsupported versions and damaged compressed files discoverable.
      let mut count = 0;
      let header = visit_session(&path, |line| {
        if is_message(&line) {
          count += 1;
        }
      })?;
      let mut reference = reference(&path, &header);
      reference.message_count = count;
      references.push(reference);
    }
    sort_refs(&mut references);
    Ok(references)
  }

  pub fn list_session_relations(&self) -> Result<Vec<SessionRef>, String> {
    let mut references = Vec::new();
    for path in self.paths()? {
      let mut reader = storage::reader(&path)?;
      let mut first = String::new();
      reader
        .read_line(&mut first)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
      references.push(reference(&path, &header(storage::parse(&first, &path, 1)?, &path)?));
    }
    sort_refs(&mut references);
    Ok(references)
  }

  pub fn load_session(&self, id_or_path: &str) -> Result<LoadedSession, String> {
    let candidate = Path::new(id_or_path);
    if candidate.is_file() {
      return self.load_session_path(candidate);
    }
    let references = self.list_session_relations()?;
    let exact: Vec<_> = references
      .iter()
      .filter(|reference| reference.id == id_or_path)
      .collect();
    let matches = if exact.is_empty() {
      references
        .iter()
        .filter(|reference| reference.id.starts_with(id_or_path))
        .collect()
    } else {
      exact
    };
    match matches.as_slice() {
      [reference] => self.load_session_path(&reference.path),
      [] => Err(format!("no dsh session found for `{id_or_path}`")),
      _ => Err(format!(
        "multiple dsh sessions match `{id_or_path}`; use an exact id or path"
      )),
    }
  }

  pub fn load_session_path(&self, path: &Path) -> Result<LoadedSession, String> {
    let mut lines = Vec::new();
    let header = visit_session(path, |line| lines.push(line))?;
    let mut reference = reference(path, &header);
    reference.message_count = message_count(&lines);
    let history_status = if header.origin.as_deref() == Some("subagent") && header.seed_length.unwrap_or(0) > 0 {
      SessionHistoryStatus::FilteredSubagent
    } else {
      SessionHistoryStatus::Complete
    };
    Ok(LoadedSession {
      reference,
      events: normalize::normalize(&header, lines),
      history_status,
    })
  }

  fn paths(&self) -> Result<Vec<PathBuf>, String> {
    let root = session_root(
      self.session_dir.clone(),
      std::env::var_os("DSH_HOME"),
      std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")),
    )?;
    let mut paths = Vec::new();
    collect(&root, &mut paths)?;
    paths.sort();
    Ok(paths)
  }
}

fn session_root(
  explicit: Option<PathBuf>,
  dsh_home: Option<std::ffi::OsString>,
  home: Option<std::ffi::OsString>,
) -> Result<PathBuf, String> {
  if let Some(root) = explicit {
    return Ok(root);
  }
  if let Some(home) = dsh_home.filter(|value| !value.is_empty()) {
    return Ok(PathBuf::from(home).join("sessions"));
  }
  home
    .map(|home| PathBuf::from(home).join(".dsh/sessions"))
    .ok_or_else(|| "set DSH_HOME, HOME, USERPROFILE, or --session-dir to locate dsh sessions".into())
}

fn collect(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
  let entries = match std::fs::read_dir(root) {
    Ok(entries) => entries,
    Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
    Err(err) => return Err(format!("failed to scan {}: {err}", root.display())),
  };
  for entry in entries {
    let entry = entry.map_err(|err| format!("failed to scan {}: {err}", root.display()))?;
    let kind = entry.file_type().map_err(|err| err.to_string())?;
    if kind.is_dir() {
      collect(&entry.path(), paths)?;
    } else if kind.is_file() && matches!(entry.file_name().to_str(), Some("session.jsonl" | "session.jsonl.zstd")) {
      paths.push(entry.path());
    }
  }
  Ok(())
}

fn header(line: DshSessionLine, path: &Path) -> Result<SessionHeader, String> {
  let DshSessionItem::Session(header) = line.into_item() else {
    return Err(format!("invalid dsh session header in {}", path.display()));
  };
  if header.version != 0 {
    return Err(format!(
      "unsupported dsh session version {} in {} (expected 0)",
      header.version,
      path.display()
    ));
  }
  Ok(header)
}

fn visit_session(path: &Path, mut visit: impl FnMut(DshSessionLine)) -> Result<SessionHeader, String> {
  let mut reader = storage::reader(path)?;
  let mut buffer = String::new();
  reader
    .read_line(&mut buffer)
    .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
  let header = header(storage::parse(&buffer, path, 1)?, path)?;
  let seed_length = if header.origin.as_deref() == Some("subagent") {
    header.seed_length.unwrap_or(0)
  } else {
    0
  };
  let mut index = 1;
  loop {
    buffer.clear();
    index += 1;
    if reader
      .read_line(&mut buffer)
      .map_err(|err| format!("failed to read {}:{index}: {err}", path.display()))?
      == 0
    {
      break;
    }
    if buffer.trim().is_empty() {
      continue;
    }
    for line in storage::expand(storage::parse(&buffer, path, index)?)
      .map_err(|err| format!("{}:{index}: {err}", path.display()))?
    {
      // seedLength is immutable fork ancestry, unlike end-seed, which also
      // appears on every resume and must not hide the session's own past turns.
      if line
        .native()
        .get("seq")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|seq| seq < seed_length)
      {
        continue;
      }
      visit(line);
    }
  }
  Ok(header)
}

fn reference(path: &Path, header: &SessionHeader) -> SessionRef {
  SessionRef {
    id: header.id.clone(),
    parent_session_id: if header.origin.as_deref() == Some("subagent") {
      header.parent_session.clone()
    } else {
      None
    },
    agent_path: None,
    agent_nickname: None,
    agent_role: None,
    path: path.to_path_buf(),
    cwd: header.cwd.clone(),
    timestamp: Some(header.created_at.to_string()),
    message_count: 0,
  }
}

fn message_count(lines: &[DshSessionLine]) -> usize {
  lines.iter().filter(|line| is_message(line)).count()
}

fn is_message(line: &DshSessionLine) -> bool {
  matches!(
    line.item(),
    DshSessionItem::Event(
      tokn_dsh_protocol::SessionEvent::UserMessage(_) | tokn_dsh_protocol::SessionEvent::AssistantMessage(_)
    )
  )
}

fn sort_refs(references: &mut [SessionRef]) {
  references.sort_by_key(|reference| {
    (
      std::cmp::Reverse(
        reference
          .timestamp
          .as_deref()
          .and_then(|value| value.parse::<u64>().ok()),
      ),
      reference.path.clone(),
    )
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn resolves_root_precedence_without_mutating_environment() {
    assert_eq!(
      session_root(Some("explicit".into()), Some("custom".into()), Some("home".into())).unwrap(),
      PathBuf::from("explicit")
    );
    assert_eq!(
      session_root(None, Some("custom".into()), Some("home".into())).unwrap(),
      PathBuf::from("custom/sessions")
    );
    assert_eq!(
      session_root(None, Some("".into()), Some("home".into())).unwrap(),
      PathBuf::from("home/.dsh/sessions")
    );
  }
}
