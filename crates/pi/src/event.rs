use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum PiEvent {
  #[serde(rename = "session")]
  Session(PiSessionEvent),
  #[serde(rename = "model_change")]
  ModelChange(PiModelChangeEvent),
  #[serde(rename = "thinking_level_change")]
  ThinkingLevelChange(PiThinkingLevelChangeEvent),
  #[serde(rename = "message")]
  Message(PiMessageEvent),
  #[serde(rename = "error")]
  Error(PiErrorEvent),
  #[serde(untagged)]
  Unknown(Value),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiSessionEvent {
  pub id: String,
  pub timestamp: Option<String>,
  pub cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiModelChangeEvent {
  pub id: Option<String>,
  pub parent_id: Option<String>,
  pub timestamp: Option<String>,
  pub provider: Option<String>,
  pub model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiThinkingLevelChangeEvent {
  pub id: Option<String>,
  pub parent_id: Option<String>,
  pub timestamp: Option<String>,
  pub thinking_level: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiMessageEvent {
  pub id: Option<String>,
  pub parent_id: Option<String>,
  pub timestamp: Option<String>,
  pub message: PiMessage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "role")]
pub enum PiMessage {
  #[serde(rename = "user")]
  User(PiUserMessage),
  #[serde(rename = "assistant")]
  Assistant(PiAssistantMessage),
  #[serde(rename = "toolResult")]
  ToolResult(PiToolResultMessage),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiUserMessage {
  pub content: PiUserContent,
  pub timestamp: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum PiUserContent {
  Text(String),
  Blocks(Vec<PiUserContentBlock>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum PiUserContentBlock {
  #[serde(rename = "text")]
  Text { text: String },
  #[serde(rename = "image")]
  Image { data: String, mime_type: String },
  #[serde(untagged)]
  Unknown(Value),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiAssistantMessage {
  pub content: Vec<PiAssistantContentBlock>,
  pub provider: Option<String>,
  pub model: Option<String>,
  pub timestamp: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum PiAssistantContentBlock {
  #[serde(rename = "text")]
  Text { text: String },
  #[serde(rename = "thinking")]
  Thinking {
    thinking: String,
    thinking_signature: Option<String>,
  },
  #[serde(rename = "toolCall")]
  ToolCall { id: String, name: String, arguments: Value },
  #[serde(untagged)]
  Unknown(Value),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiToolResultMessage {
  pub tool_call_id: String,
  pub tool_name: String,
  pub content: Vec<PiToolResultContentBlock>,
  pub details: Option<Value>,
  pub is_error: bool,
  pub timestamp: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum PiToolResultContentBlock {
  #[serde(rename = "text")]
  Text { text: String },
  #[serde(rename = "image")]
  Image { data: String, mime_type: String },
  #[serde(untagged)]
  Unknown(Value),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiErrorEvent {
  pub timestamp: Option<String>,
  pub message: Option<String>,
  pub error: Option<Value>,
}
