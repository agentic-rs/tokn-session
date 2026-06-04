use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum CodexEvent {
  SessionMeta(CodexSessionMeta),
  ResponseItem(CodexResponseItem),
  EventMsg(CodexEventMsg),
  TurnContext(Value),
  Compacted(CodexCompacted),
}

#[derive(Debug, Deserialize)]
pub struct CodexLine {
  pub timestamp: Option<String>,
  #[serde(flatten)]
  pub event: CodexEvent,
}

#[derive(Debug, Deserialize)]
pub struct CodexSessionMeta {
  pub id: String,
  pub timestamp: Option<String>,
  pub cwd: Option<String>,
  pub model_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodexResponseItem {
  Message {
    id: Option<String>,
    role: String,
    content: Vec<CodexContentItem>,
    phase: Option<String>,
  },
  Reasoning {
    id: Option<String>,
    summary: Vec<CodexReasoningSummary>,
    content: Option<Vec<CodexReasoningContent>>,
    encrypted_content: Option<String>,
  },
  FunctionCall {
    id: Option<String>,
    name: String,
    namespace: Option<String>,
    arguments: String,
    call_id: String,
  },
  FunctionCallOutput {
    call_id: String,
    output: Value,
  },
  LocalShellCall {
    id: Option<String>,
    call_id: Option<String>,
    status: Option<String>,
    action: Value,
  },
  CustomToolCall {
    id: Option<String>,
    status: Option<String>,
    call_id: String,
    name: String,
    input: String,
  },
  CustomToolCallOutput {
    call_id: String,
    name: Option<String>,
    output: Value,
  },
  WebSearchCall {
    id: Option<String>,
    call_id: Option<String>,
    status: Option<String>,
    action: Value,
  },
  ToolSearchCall {
    id: Option<String>,
    call_id: Option<String>,
    status: Option<String>,
    execution: String,
    arguments: Value,
  },
  ToolSearchOutput {
    call_id: Option<String>,
    status: String,
    execution: String,
    tools: Vec<Value>,
  },
  #[serde(untagged)]
  Unknown(Value),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodexContentItem {
  InputText {
    text: String,
  },
  OutputText {
    text: String,
  },
  Text {
    text: String,
  },
  #[serde(untagged)]
  Unknown(Value),
}

#[derive(Debug, Deserialize)]
pub struct CodexReasoningSummary {
  pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodexReasoningContent {
  ReasoningText {
    text: String,
  },
  Text {
    text: String,
  },
  #[serde(untagged)]
  Unknown(Value),
}

#[derive(Debug, Deserialize)]
pub struct CodexCompacted {
  pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodexEventMsg {
  TaskStarted {
    turn_id: Option<String>,
  },
  TaskComplete {
    turn_id: Option<String>,
  },
  UserMessage {
    message: String,
  },
  AgentMessage {
    message: String,
    phase: Option<String>,
  },
  AgentReasoning {
    text: Option<String>,
    message: Option<String>,
  },
  ExecCommandBegin {
    call_id: Option<String>,
    command: Vec<String>,
  },
  ExecCommandEnd {
    call_id: Option<String>,
    status: Option<String>,
  },
  McpToolCallBegin {
    call_id: Option<String>,
    name: Option<String>,
    arguments: Option<Value>,
  },
  McpToolCallEnd {
    call_id: Option<String>,
    name: Option<String>,
    result: Option<Value>,
    error: Option<Value>,
  },
  Error {
    message: String,
  },
  PatchApplyBegin {
    call_id: String,
    changes: Value,
  },
  PatchApplyEnd {
    call_id: String,
    stdout: String,
    stderr: String,
    success: bool,
    changes: Option<Value>,
    status: String,
  },
  TokenCount {},
  ThreadGoalUpdated {
    #[serde(rename = "threadId")]
    thread_id: Option<String>,
    #[serde(rename = "turnId")]
    turn_id: Option<String>,
    goal: Option<Value>,
  },
  TurnStarted {
    turn_id: Option<String>,
  },
  TurnComplete {},
  TurnAborted {
    reason: Option<String>,
  },
  #[serde(untagged)]
  Unknown(Value),
}
