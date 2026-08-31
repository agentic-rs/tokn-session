use std::path::PathBuf;

use crate::agent_event::AgentEvent;

/// Metadata used to discover a session without reading its conversation body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionHeader {
  pub id: String,
  pub parent_session_id: Option<String>,
  pub agent_path: Option<String>,
  pub agent_nickname: Option<String>,
  pub agent_role: Option<String>,
  /// Provider-native user-facing title, when the provider persists one.
  pub title: Option<String>,
  /// Best available first-message preview. This remains separate from
  /// `title` so clients can choose an explicit title before prompt text.
  pub preview: Option<String>,
  pub path: PathBuf,
  pub cwd: Option<String>,
  /// Provider-native session creation time, retained without reinterpretation.
  pub timestamp: Option<String>,
  /// Last-update time. File-backed providers expose file mtime as Unix
  /// milliseconds; catalog-backed providers retain their native update value.
  pub updated_at: Option<String>,
  /// Canonical Unix milliseconds used for cross-provider ordering.
  pub updated_at_ms: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct SessionRef {
  pub id: String,
  pub parent_session_id: Option<String>,
  pub agent_path: Option<String>,
  pub agent_nickname: Option<String>,
  pub agent_role: Option<String>,
  pub title: Option<String>,
  pub preview: Option<String>,
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
