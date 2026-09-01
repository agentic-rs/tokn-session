use tokn_session_client::{AgentClient, SessionHeader};
use tokn_session_core::LoadedSession;

use crate::model::{SessionLocator, ViewerProvider};

pub(crate) trait ViewerRepository: Send + Sync {
  fn list_session_headers(&self, provider: ViewerProvider) -> Result<Vec<SessionHeader>, String>;
  fn load_session(&self, locator: &SessionLocator) -> Result<LoadedSession, String>;
}

pub(crate) struct NativeRepository;

impl ViewerRepository for NativeRepository {
  fn list_session_headers(&self, provider: ViewerProvider) -> Result<Vec<SessionHeader>, String> {
    AgentClient::list_session_headers(provider.source(), None)
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
