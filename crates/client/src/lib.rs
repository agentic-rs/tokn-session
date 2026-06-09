mod print;
mod session_source;
mod source;

use std::path::PathBuf;

use tokn_session_core::{LoadedSession, SessionRef};

pub use print::{AppendAction, AppendSessionRequest, CreateSessionRequest};
pub use source::Source;

pub struct AgentClient;

impl AgentClient {
  pub fn list_sessions(source: Source, session_dir: Option<PathBuf>) -> Result<Vec<SessionRef>, String> {
    session_source::session_source(source, session_dir)?.list_sessions()
  }

  pub fn load_session(source: Source, session_dir: Option<PathBuf>, session: &str) -> Result<LoadedSession, String> {
    session_source::session_source(source, session_dir)?.load_session(session)
  }

  pub fn create_session(request: CreateSessionRequest) -> Result<(), String> {
    print::create_session(request)
  }

  pub fn append_session(request: AppendSessionRequest) -> Result<(), String> {
    print::append_session(request)
  }
}
