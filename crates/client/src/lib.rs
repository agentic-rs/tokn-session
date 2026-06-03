use std::path::PathBuf;

use tokn_agent_codex::CodexSessionSource;
use tokn_agent_core::{LoadedSession, SessionRef};
use tokn_agent_opencode::OpenCodeSessionSource;
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
  Codex(CodexSessionSource),
  OpenCode(OpenCodeSessionSource),
  Pi(PiSessionSource),
}

impl SessionSourceClient {
  fn list_sessions(&self) -> Result<Vec<SessionRef>, String> {
    match self {
      Self::Codex(source) => source.list_sessions(),
      Self::OpenCode(source) => source.list_sessions(),
      Self::Pi(source) => source.list_sessions(),
    }
  }

  fn load_session(&self, session: &str) -> Result<LoadedSession, String> {
    match self {
      Self::Codex(source) => source.load_session(session),
      Self::OpenCode(source) => source.load_session(session),
      Self::Pi(source) => source.load_session(session),
    }
  }
}

fn session_source(source: Source, session_dir: Option<PathBuf>) -> Result<SessionSourceClient, String> {
  match source {
    Source::Pi => Ok(SessionSourceClient::Pi(PiSessionSource::new(session_dir))),
    Source::Codex => Ok(SessionSourceClient::Codex(CodexSessionSource::new(session_dir))),
    Source::OpenCode => Ok(SessionSourceClient::OpenCode(OpenCodeSessionSource::new(session_dir))),
  }
}
