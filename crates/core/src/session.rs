use std::path::PathBuf;

use crate::agent_event::AgentEvent;

#[derive(Clone, Debug)]
pub struct SessionRef {
  pub id: String,
  pub parent_session_id: Option<String>,
  pub agent_path: Option<String>,
  pub agent_nickname: Option<String>,
  pub agent_role: Option<String>,
  pub path: PathBuf,
  pub cwd: Option<String>,
  pub timestamp: Option<String>,
  pub message_count: usize,
}

#[derive(Debug)]
pub struct LoadedSession {
  pub reference: SessionRef,
  pub events: Vec<AgentEvent>,
  pub history_status: SessionHistoryStatus,
}

#[derive(Debug)]
pub struct LoadedSessionTree {
  pub session: LoadedSession,
  pub children: Vec<LoadedSessionTree>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionHistoryStatus {
  Complete,
  FilteredSubagent,
  SubagentBodyUnavailable,
}
