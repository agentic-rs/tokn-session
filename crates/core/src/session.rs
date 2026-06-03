use std::path::PathBuf;

use crate::agent_event::AgentEvent;

#[derive(Debug)]
pub struct SessionRef {
  pub id: String,
  pub path: PathBuf,
  pub cwd: Option<String>,
  pub timestamp: Option<String>,
  pub message_count: usize,
}

#[derive(Debug)]
pub struct LoadedSession {
  pub reference: SessionRef,
  pub events: Vec<AgentEvent>,
}
