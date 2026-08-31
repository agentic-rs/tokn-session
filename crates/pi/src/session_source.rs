use std::ffi::OsStr;
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

    let configured_session_dir = std::env::var_os("PI_CODING_AGENT_SESSION_DIR");
    let configured_agent_dir = std::env::var_os("PI_CODING_AGENT_DIR");
    let platform_home = dirs::home_dir();
    resolve_pi_session_root(
      self.session_dir.as_deref(),
      configured_session_dir.as_deref(),
      configured_agent_dir.as_deref(),
      platform_home.as_deref(),
      cfg!(windows),
    )
  }
}

fn resolve_pi_session_root(
  explicit_session_dir: Option<&Path>,
  configured_session_dir: Option<&OsStr>,
  configured_agent_dir: Option<&OsStr>,
  platform_home: Option<&Path>,
  is_windows: bool,
) -> Result<PathBuf, String> {
  if let Some(explicit_session_dir) = explicit_session_dir {
    return Ok(explicit_session_dir.to_path_buf());
  }

  if let Some(configured_session_dir) = configured_session_dir.filter(|value| !value.is_empty()) {
    return expand_pi_tilde(
      configured_session_dir,
      platform_home,
      "PI_CODING_AGENT_SESSION_DIR",
      is_windows,
    );
  }

  if let Some(configured_agent_dir) = configured_agent_dir.filter(|value| !value.is_empty()) {
    return expand_pi_tilde(configured_agent_dir, platform_home, "PI_CODING_AGENT_DIR", is_windows)
      .map(|agent_dir| agent_dir.join("sessions"));
  }

  let platform_home = platform_home
    .filter(|path| !path.as_os_str().is_empty())
    .ok_or_else(|| {
      "could not determine the user home directory; set PI_CODING_AGENT_SESSION_DIR, PI_CODING_AGENT_DIR, or pass --session-dir"
        .to_string()
    })?;
  Ok(platform_home.join(".pi").join("agent").join("sessions"))
}

fn expand_pi_tilde(
  value: &OsStr,
  platform_home: Option<&Path>,
  variable: &str,
  is_windows: bool,
) -> Result<PathBuf, String> {
  let Some(value) = value.to_str() else {
    return Ok(PathBuf::from(value));
  };
  let suffix = if value == "~" {
    Some("")
  } else if let Some(suffix) = value.strip_prefix("~/") {
    Some(suffix)
  } else if is_windows {
    value.strip_prefix("~\\")
  } else {
    None
  };

  let Some(suffix) = suffix else {
    return Ok(PathBuf::from(value));
  };
  let platform_home = platform_home.ok_or_else(|| {
    format!(
      "{variable} uses `~`, but the user home directory could not be determined; set {variable} to a path without `~` or pass --session-dir"
    )
  })?;

  if suffix.is_empty() {
    Ok(platform_home.to_path_buf())
  } else {
    Ok(platform_home.join(suffix))
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

  #[test]
  fn explicit_session_directory_has_highest_precedence_and_is_unchanged() {
    let explicit = Path::new("~/explicit-sessions");

    let resolved = resolve_pi_session_root(
      Some(explicit),
      Some(OsStr::new("environment-sessions")),
      Some(OsStr::new("environment-agent")),
      Some(Path::new("platform-home")),
      false,
    )
    .expect("explicit session directory should resolve");

    assert_eq!(resolved, explicit);
  }

  #[test]
  fn session_environment_override_precedes_agent_directory() {
    let resolved = resolve_pi_session_root(
      None,
      Some(OsStr::new("environment-sessions")),
      Some(OsStr::new("environment-agent")),
      Some(Path::new("platform-home")),
      false,
    )
    .expect("session environment override should resolve");

    assert_eq!(resolved, Path::new("environment-sessions"));
  }

  #[test]
  fn empty_session_override_falls_back_to_agent_directory() {
    let resolved = resolve_pi_session_root(
      None,
      Some(OsStr::new("")),
      Some(OsStr::new("environment-agent")),
      Some(Path::new("platform-home")),
      false,
    )
    .expect("agent directory should resolve");

    assert_eq!(resolved, Path::new("environment-agent").join("sessions"));
  }

  #[test]
  fn empty_environment_overrides_use_cross_platform_home_input() {
    // At runtime dirs::home_dir supplies this value from the Windows profile
    // known folder, rather than this crate depending on HOME being present.
    let user_profile = Path::new(r"C:\Users\Alice");
    let resolved = resolve_pi_session_root(
      None,
      Some(OsStr::new("")),
      Some(OsStr::new("")),
      Some(user_profile),
      true,
    )
    .expect("platform home should resolve");

    assert_eq!(resolved, user_profile.join(".pi").join("agent").join("sessions"));
  }

  #[test]
  fn environment_overrides_expand_tilde_with_platform_home() {
    let platform_home = Path::new("platform-home");
    let session_root = resolve_pi_session_root(
      None,
      Some(OsStr::new("~/custom-sessions")),
      None,
      Some(platform_home),
      false,
    )
    .expect("session override should expand");
    let agent_root = resolve_pi_session_root(
      None,
      None,
      Some(OsStr::new("~/custom-agent")),
      Some(platform_home),
      false,
    )
    .expect("agent override should expand");

    assert_eq!(session_root, platform_home.join("custom-sessions"));
    assert_eq!(agent_root, platform_home.join("custom-agent").join("sessions"));
  }

  #[test]
  fn windows_tilde_separator_uses_platform_home() {
    let platform_home = Path::new(r"C:\Users\Alice");

    let resolved = resolve_pi_session_root(
      None,
      Some(OsStr::new(r"~\custom-sessions")),
      None,
      Some(platform_home),
      true,
    )
    .expect("Windows tilde path should expand");

    assert_eq!(resolved, platform_home.join("custom-sessions"));
  }

  #[test]
  fn missing_platform_home_has_actionable_errors() {
    let default_error =
      resolve_pi_session_root(None, None, None, None, false).expect_err("default discovery requires a platform home");
    let tilde_error = resolve_pi_session_root(None, Some(OsStr::new("~/custom-sessions")), None, None, false)
      .expect_err("tilde expansion requires a platform home");

    assert!(default_error.contains("PI_CODING_AGENT_SESSION_DIR"));
    assert!(default_error.contains("--session-dir"));
    assert!(tilde_error.contains("PI_CODING_AGENT_SESSION_DIR"));
    assert!(tilde_error.contains("--session-dir"));
  }

  fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
  }
}
