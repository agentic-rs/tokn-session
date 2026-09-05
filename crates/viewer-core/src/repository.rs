use std::path::{Path, PathBuf};

use tokn_session_client::{AgentClient, SessionHeader};
use tokn_session_core::LoadedSession;

use crate::model::{SessionLocator, ViewerProvider};

pub(crate) trait ViewerRepository: Send + Sync {
  fn list_session_headers(&self, provider: ViewerProvider) -> Result<Vec<SessionHeader>, String>;

  /// Resolves file roots for providers that can be incrementally cataloged
  /// from individual filesystem changes. The default keeps existing test and
  /// alternate repositories source-compatible until they opt in.
  fn file_session_roots(&self, provider: ViewerProvider) -> Result<Vec<PathBuf>, String> {
    Err(format!(
      "{} does not support path-targeted session cataloging",
      provider.as_str()
    ))
  }

  /// Reads one metadata-only session header from a known changed file. The
  /// default makes unsupported repository implementations fail closed, so a
  /// caller can fall back to a complete provider catalog instead.
  fn session_header_at_path(&self, provider: ViewerProvider, path: &Path) -> Result<SessionHeader, String> {
    Err(format!(
      "{} does not support path-targeted session cataloging for {}",
      provider.as_str(),
      path.display()
    ))
  }

  /// Avoids parsing a JSONL file while an agent is actively appending to it.
  /// Alternate repositories are stable by construction unless they opt in.
  fn session_body_ready(&self, _locator: &SessionLocator) -> Result<bool, String> {
    Ok(true)
  }

  fn load_session(&self, locator: &SessionLocator) -> Result<LoadedSession, String>;
}

pub(crate) struct NativeRepository;

impl ViewerRepository for NativeRepository {
  fn list_session_headers(&self, provider: ViewerProvider) -> Result<Vec<SessionHeader>, String> {
    AgentClient::list_session_headers(provider.source(), None)
  }

  fn file_session_roots(&self, provider: ViewerProvider) -> Result<Vec<PathBuf>, String> {
    AgentClient::file_session_roots(provider.source(), None)
  }

  fn session_header_at_path(&self, provider: ViewerProvider, path: &Path) -> Result<SessionHeader, String> {
    AgentClient::session_header_at_path(provider.source(), None, path)
  }

  fn session_body_ready(&self, locator: &SessionLocator) -> Result<bool, String> {
    if !matches!(locator.provider, ViewerProvider::Codex | ViewerProvider::Pi) {
      return Ok(true);
    }
    let modified = std::fs::metadata(&locator.source_path)
      .and_then(|metadata| metadata.modified())
      .map_err(|error| format!("failed to inspect {}: {error}", locator.source_path.display()))?;
    Ok(
      modified
        .elapsed()
        .is_ok_and(|age| age >= std::time::Duration::from_secs(2)),
    )
  }

  fn load_session(&self, locator: &SessionLocator) -> Result<LoadedSession, String> {
    match locator.provider {
      ViewerProvider::OpenCode | ViewerProvider::ZCode => AgentClient::load_session(
        locator.provider.source(),
        Some(locator.source_path.clone()),
        &locator.session_id,
      ),
      ViewerProvider::Codex | ViewerProvider::Pi | ViewerProvider::WorkBuddy | ViewerProvider::Dsh => {
        let path = locator
          .source_path
          .to_str()
          .ok_or_else(|| "session path is not valid UTF-8".to_string())?;
        AgentClient::load_session(locator.provider.source(), None, path)
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn native_repository_defers_an_active_jsonl_body() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("active.jsonl");
    std::fs::write(&path, "active").unwrap();
    let locator = SessionLocator {
      version: 1,
      provider: ViewerProvider::Codex,
      session_id: "active".into(),
      source_path: path,
    };
    assert!(!NativeRepository.session_body_ready(&locator).unwrap());
  }
}
