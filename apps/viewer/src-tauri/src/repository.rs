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
