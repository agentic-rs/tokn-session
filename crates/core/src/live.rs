use serde::Serialize;

use crate::agent_event::{AgentEvent, Provider, UnknownEvent};

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveSessionEvent {
  Started(LiveSessionStarted),
  Event(AgentEvent),
  Finished(LiveSessionFinished),
  Unknown(UnknownEvent),
}

#[derive(Debug, Serialize)]
pub struct LiveSessionStarted {
  pub provider: Provider,
  pub session_id: String,
  pub cwd: Option<String>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LiveSessionFinished {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub success: bool,
  pub exit_code: Option<i32>,
  pub timestamp: Option<String>,
}
