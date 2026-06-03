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
    Unknown(PiUnknownEvent),
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
#[serde(rename_all = "camelCase")]
pub struct PiMessage {
    pub role: String,
    pub content: Value,
    pub timestamp: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiErrorEvent {
    pub timestamp: Option<String>,
    pub message: Option<String>,
    pub error: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct PiUnknownEvent {
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub timestamp: Option<String>,
}
