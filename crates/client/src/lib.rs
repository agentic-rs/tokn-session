use std::path::PathBuf;

use tokn_agent_core::{LoadedSession, SessionRef};
use tokn_agent_pi::PiSessionSource;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
  Pi,
  Codex,
  OpenCode,
}

pub struct AgentClient;

impl AgentClient {
  pub fn list_sessions(source: Source, session_dir: Option<PathBuf>) -> Result<Vec<SessionRef>, String> {
    session_source(source, session_dir)?.list_sessions()
  }

  pub fn load_session(source: Source, session_dir: Option<PathBuf>, session: &str) -> Result<LoadedSession, String> {
    session_source(source, session_dir)?.load_session(session)
  }
}

enum SessionSourceClient {
  Pi(PiSessionSource),
}

impl SessionSourceClient {
  fn list_sessions(&self) -> Result<Vec<SessionRef>, String> {
    match self {
      Self::Pi(source) => source.list_sessions(),
    }
  }

  fn load_session(&self, session: &str) -> Result<LoadedSession, String> {
    match self {
      Self::Pi(source) => source.load_session(session),
    }
  }
}

fn session_source(source: Source, session_dir: Option<PathBuf>) -> Result<SessionSourceClient, String> {
  match source {
    Source::Pi => Ok(SessionSourceClient::Pi(PiSessionSource::new(session_dir))),
    Source::Codex => Err("codex sessions are not implemented yet".to_string()),
    Source::OpenCode => Err("opencode sessions are not implemented yet".to_string()),
  }
}
