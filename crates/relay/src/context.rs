use std::path::Path;

use serde::Serialize;
use serde_json::Value;
use tokn_session_core::Provider;

#[derive(Clone, Debug, Serialize)]
pub struct ProjectContext {
  pub id: Option<String>,
  pub name: Option<String>,
  pub folder: Option<String>,
  pub repository_url: Option<String>,
  pub branch: Option<String>,
  pub commit_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionContext {
  pub provider: Provider,
  pub session_id: String,
  pub parent_session_id: Option<String>,
  pub title: Option<String>,
  pub cwd: Option<String>,
  pub started_at: Option<String>,
  pub project: Option<ProjectContext>,
}

impl SessionContext {
  pub(crate) fn from_path(provider: Provider, path: &Path) -> Self {
    Self {
      provider,
      session_id: session_id_from_path(path),
      parent_session_id: None,
      title: None,
      cwd: None,
      started_at: None,
      project: None,
    }
  }

  pub(crate) fn update(&mut self, value: &Value) {
    match self.provider {
      Provider::Codex => self.update_codex(value),
      Provider::Pi => self.update_pi(value),
      Provider::OpenCode => {}
    }
  }

  fn update_codex(&mut self, value: &Value) {
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
      return;
    }
    let Some(payload) = value.get("payload") else {
      return;
    };

    if let Some(id) = string_field(payload, "id") {
      self.session_id = id;
    }
    self.parent_session_id = first_string_field(payload, &["parent_thread_id", "forked_from_id"]);
    self.title = string_field(payload, "title");
    self.cwd = string_field(payload, "cwd");
    self.started_at = string_field(payload, "timestamp").or_else(|| string_field(value, "timestamp"));
    self.project = project_context(self.cwd.as_deref(), payload.get("git"));
  }

  fn update_pi(&mut self, value: &Value) {
    if value.get("type").and_then(Value::as_str) != Some("session") {
      return;
    }

    if let Some(id) = string_field(value, "id") {
      self.session_id = id;
    }
    self.title = string_field(value, "title");
    self.cwd = string_field(value, "cwd");
    self.started_at = string_field(value, "timestamp");
    self.project = project_context(self.cwd.as_deref(), None);
  }
}

fn project_context(cwd: Option<&str>, git: Option<&Value>) -> Option<ProjectContext> {
  let repository_url = git.and_then(|git| string_field(git, "repository_url"));
  let folder = cwd.map(str::to_string);
  let name = repository_url
    .as_deref()
    .and_then(project_name_from_repository)
    .or_else(|| cwd.and_then(project_name_from_folder));
  let branch = git.and_then(|git| string_field(git, "branch"));
  let commit_hash = git.and_then(|git| string_field(git, "commit_hash"));

  if repository_url.is_none() && folder.is_none() && branch.is_none() && commit_hash.is_none() {
    return None;
  }

  Some(ProjectContext {
    id: repository_url.clone(),
    name,
    folder,
    repository_url,
    branch,
    commit_hash,
  })
}

fn project_name_from_repository(repository_url: &str) -> Option<String> {
  repository_url
    .trim_end_matches('/')
    .rsplit(['/', ':'])
    .next()
    .map(|name| name.trim_end_matches(".git"))
    .filter(|name| !name.is_empty())
    .map(str::to_string)
}

fn project_name_from_folder(folder: &str) -> Option<String> {
  Path::new(folder)
    .file_name()
    .and_then(|name| name.to_str())
    .filter(|name| !name.is_empty())
    .map(str::to_string)
}

pub(crate) fn session_id_from_path(path: &Path) -> String {
  path
    .file_stem()
    .and_then(|value| value.to_str())
    .unwrap_or("unknown")
    .rsplit(['-', '_'])
    .next()
    .unwrap_or("unknown")
    .to_string()
}

fn first_string_field(value: &Value, fields: &[&str]) -> Option<String> {
  fields.iter().find_map(|field| string_field(value, field))
}

fn string_field(value: &Value, field: &str) -> Option<String> {
  value
    .get(field)
    .and_then(Value::as_str)
    .filter(|value| !value.is_empty())
    .map(str::to_string)
}

#[cfg(test)]
mod tests {
  use std::path::Path;

  use serde_json::json;
  use tokn_session_core::Provider;

  use super::SessionContext;

  #[test]
  fn extracts_codex_session_and_project_context() {
    let mut context = SessionContext::from_path(Provider::Codex, Path::new("fallback.jsonl"));
    context.update(&json!({
      "timestamp": "2026-06-04T00:00:00Z",
      "type": "session_meta",
      "payload": {
        "id": "session-1",
        "parent_thread_id": "parent-1",
        "title": "Investigate relay",
        "cwd": "/tmp/worktree",
        "git": {
          "repository_url": "git@github.com:agentic-rs/tokn-session.git",
          "branch": "main",
          "commit_hash": "abcdef"
        }
      }
    }));

    assert_eq!(context.session_id, "session-1");
    assert_eq!(context.parent_session_id.as_deref(), Some("parent-1"));
    assert_eq!(context.title.as_deref(), Some("Investigate relay"));
    assert_eq!(context.started_at.as_deref(), Some("2026-06-04T00:00:00Z"));
    let project = context.project.unwrap();
    assert_eq!(project.name.as_deref(), Some("tokn-session"));
    assert_eq!(project.folder.as_deref(), Some("/tmp/worktree"));
  }
}
