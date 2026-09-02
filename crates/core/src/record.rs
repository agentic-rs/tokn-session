use serde::Serialize;
use serde_json::Value;

use crate::{AgentEvent, LoadedSession, SessionHistoryStatus, SessionRef};

/// One source record and its ordered normalization output. IDs are scoped to
/// the source path and session. A record can normalize to no events.
#[derive(Debug, Serialize)]
pub struct NormalizedRecord {
  pub record_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub native: Option<Value>,
  pub events: Vec<AgentEvent>,
}

#[derive(Debug)]
pub struct LoadedSessionRecords {
  pub reference: SessionRef,
  pub records: Vec<NormalizedRecord>,
  pub history_status: SessionHistoryStatus,
}

impl From<LoadedSessionRecords> for LoadedSession {
  fn from(loaded: LoadedSessionRecords) -> Self {
    Self {
      reference: loaded.reference,
      events: loaded.records.into_iter().flat_map(|record| record.events).collect(),
      history_status: loaded.history_status,
    }
  }
}
