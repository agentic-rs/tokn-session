use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
  SessionStarted(SessionStarted),
  ProviderChanged(ProviderChanged),
  Message(MessageEvent),
  Reasoning(ReasoningEvent),
  GoalUpdated(GoalUpdated),
  ToolCall(ToolCallEvent),
  Error(ErrorEvent),
  Unknown(UnknownEvent),
}

#[derive(Debug, Serialize)]
pub struct SessionStarted {
  pub provider: Provider,
  pub session_id: String,
  pub cwd: Option<String>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderChanged {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub native_id: Option<String>,
  pub native_parent_id: Option<String>,
  pub model_provider: Option<String>,
  pub model_id: Option<String>,
  pub thinking_level: Option<String>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MessageEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub message_id: Option<String>,
  pub parent_id: Option<String>,
  pub role: Role,
  pub phase: Phase,
  pub text: String,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReasoningEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub message_id: Option<String>,
  pub parent_id: Option<String>,
  pub phase: Phase,
  pub text: Option<String>,
  pub summary: Option<String>,
  pub encrypted_content: Option<String>,
  pub signature: Option<String>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GoalUpdated {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub turn_id: Option<String>,
  pub goal: Option<Value>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolCallEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub message_id: Option<String>,
  pub parent_id: Option<String>,
  pub tool_call_id: Option<String>,
  pub tool_name: Option<String>,
  pub phase: Phase,
  pub input: Option<Value>,
  pub output: Option<Value>,
  pub is_error: Option<bool>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ErrorEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub message: String,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UnknownEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub native_type: Option<String>,
  pub timestamp: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
  Pi,
  Codex,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum Role {
  User,
  Assistant,
  System,
  Tool,
  Unknown,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum Phase {
  Started,
  Delta,
  Updated,
  Finished,
}
