use std::path::PathBuf;

use tokn_session_codex::CodexSessionSource;
use tokn_session_core::{LoadedSession, SessionRef};
use tokn_session_opencode::OpenCodeSessionSource;
use tokn_session_pi::PiSessionSource;

use crate::Source;

pub(crate) enum SessionSourceClient {
  Codex(CodexSessionSource),
  OpenCode(OpenCodeSessionSource),
  Pi(PiSessionSource),
}

impl SessionSourceClient {
  pub(crate) fn list_sessions(&self) -> Result<Vec<SessionRef>, String> {
    match self {
      Self::Codex(source) => source.list_sessions(),
      Self::OpenCode(source) => source.list_sessions(),
      Self::Pi(source) => source.list_sessions(),
    }
  }

  pub(crate) fn load_session(&self, session: &str) -> Result<LoadedSession, String> {
    match self {
      Self::Codex(source) => source.load_session(session),
      Self::OpenCode(source) => source.load_session(session),
      Self::Pi(source) => source.load_session(session),
    }
  }
}

pub(crate) fn session_source(source: Source, session_dir: Option<PathBuf>) -> Result<SessionSourceClient, String> {
  match source {
    Source::Pi => Ok(SessionSourceClient::Pi(PiSessionSource::new(session_dir))),
    Source::Codex => Ok(SessionSourceClient::Codex(CodexSessionSource::new(session_dir))),
    Source::OpenCode => Ok(SessionSourceClient::OpenCode(OpenCodeSessionSource::new(session_dir))),
  }
}
