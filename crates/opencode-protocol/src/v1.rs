//! OpenCode V1 message and part payloads.
//!
//! OpenCode persists these values in the `message.data` and `part.data`
//! SQLite columns. Relational identity and row timestamps are intentionally
//! outside this module.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::{ExtraFields, UnknownItem, string_field};

/// One JSON value from OpenCode's V1 `message.data` column.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageData {
  item: MessageItem,
  native: Value,
}

impl MessageData {
  pub fn native_role(&self) -> Option<&str> {
    self.item.native_role()
  }

  pub fn item(&self) -> &MessageItem {
    &self.item
  }

  pub fn into_item(self) -> MessageItem {
    self.item
  }

  pub fn native(&self) -> &Value {
    &self.native
  }

  pub fn into_native(self) -> Value {
    self.native
  }

  pub fn into_parts(self) -> (MessageItem, Value) {
    (self.item, self.native)
  }
}

impl<'de> Deserialize<'de> for MessageData {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let native = Value::deserialize(deserializer)?;
    let native_role = string_field(&native, "role");
    let item = decode_message(native_role, native.clone());
    Ok(Self { item, native })
  }
}

impl Serialize for MessageData {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    self.native.serialize(serializer)
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageItem {
  User(Box<UserMessage>),
  Assistant(Box<AssistantMessage>),
  Unknown(UnknownItem),
}

impl MessageItem {
  pub fn native_role(&self) -> Option<&str> {
    match self {
      Self::User(_) => Some("user"),
      Self::Assistant(_) => Some("assistant"),
      Self::Unknown(item) => item.native_type.as_deref(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserMessage {
  #[serde(default)]
  pub time: Option<MessageTime>,
  #[serde(default)]
  pub format: Option<Value>,
  #[serde(default)]
  pub summary: Option<Value>,
  #[serde(default)]
  pub agent: Option<String>,
  #[serde(default)]
  pub model: Option<MessageModel>,
  #[serde(default)]
  pub system: Option<String>,
  #[serde(default)]
  pub tools: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
  #[serde(default, rename = "parentID")]
  pub parent_id: Option<String>,
  #[serde(default, rename = "modelID")]
  pub model_id: Option<String>,
  #[serde(default, rename = "providerID")]
  pub provider_id: Option<String>,
  #[serde(default)]
  pub time: Option<MessageTime>,
  #[serde(default)]
  pub error: Option<Value>,
  #[serde(default)]
  pub mode: Option<String>,
  #[serde(default)]
  pub agent: Option<String>,
  #[serde(default)]
  pub path: Option<MessagePath>,
  #[serde(default)]
  pub summary: Option<bool>,
  #[serde(default)]
  pub cost: Option<f64>,
  #[serde(default)]
  pub tokens: Option<TokenUsage>,
  #[serde(default)]
  pub structured: Option<Value>,
  #[serde(default)]
  pub variant: Option<String>,
  #[serde(default)]
  pub finish: Option<String>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MessageTime {
  #[serde(default)]
  pub created: Option<u64>,
  #[serde(default)]
  pub completed: Option<u64>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MessageModel {
  #[serde(default, rename = "providerID")]
  pub provider_id: Option<String>,
  #[serde(default, rename = "modelID")]
  pub model_id: Option<String>,
  #[serde(default)]
  pub variant: Option<String>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

/// The JSON value stored in OpenCode's optional `session.model` column.
/// Database versions without that column keep model selection in messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionModel {
  #[serde(default)]
  pub id: Option<String>,
  #[serde(default, rename = "providerID")]
  pub provider_id: Option<String>,
  #[serde(default)]
  pub variant: Option<String>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MessagePath {
  #[serde(default)]
  pub cwd: Option<String>,
  #[serde(default)]
  pub root: Option<String>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
  #[serde(default)]
  pub total: Option<f64>,
  #[serde(default)]
  pub input: Option<f64>,
  #[serde(default)]
  pub output: Option<f64>,
  #[serde(default)]
  pub reasoning: Option<f64>,
  #[serde(default)]
  pub cache: Option<CacheUsage>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CacheUsage {
  #[serde(default)]
  pub read: Option<f64>,
  #[serde(default)]
  pub write: Option<f64>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

/// One JSON value from OpenCode's V1 `part.data` column, or a hydrated V1 part
/// embedded in a run event.
#[derive(Debug, Clone, PartialEq)]
pub struct PartData {
  item: PartItem,
  native: Value,
}

impl PartData {
  pub fn native_type(&self) -> Option<&str> {
    self.item.native_type()
  }

  pub fn id(&self) -> Option<&str> {
    self.native.get("id").and_then(Value::as_str)
  }

  pub fn session_id(&self) -> Option<&str> {
    self.native.get("sessionID").and_then(Value::as_str)
  }

  pub fn message_id(&self) -> Option<&str> {
    self.native.get("messageID").and_then(Value::as_str)
  }

  pub fn item(&self) -> &PartItem {
    &self.item
  }

  pub fn into_item(self) -> PartItem {
    self.item
  }

  pub fn native(&self) -> &Value {
    &self.native
  }

  pub fn into_native(self) -> Value {
    self.native
  }

  pub fn into_parts(self) -> (PartItem, Value) {
    (self.item, self.native)
  }
}

impl<'de> Deserialize<'de> for PartData {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let native = Value::deserialize(deserializer)?;
    let native_type = string_field(&native, "type");
    let item = decode_part(native_type, native.clone());
    Ok(Self { item, native })
  }
}

impl Serialize for PartData {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    self.native.serialize(serializer)
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PartItem {
  Snapshot(SnapshotPart),
  Patch(PatchPart),
  Text(TextPart),
  Reasoning(ReasoningPart),
  File(FilePart),
  Agent(AgentPart),
  Compaction(CompactionPart),
  Subtask(SubtaskPart),
  Retry(RetryPart),
  StepStart(StepStartPart),
  StepFinish(StepFinishPart),
  Tool(ToolPart),
  Unknown(UnknownItem),
}

impl PartItem {
  pub fn native_type(&self) -> Option<&str> {
    match self {
      Self::Snapshot(_) => Some("snapshot"),
      Self::Patch(_) => Some("patch"),
      Self::Text(_) => Some("text"),
      Self::Reasoning(_) => Some("reasoning"),
      Self::File(_) => Some("file"),
      Self::Agent(_) => Some("agent"),
      Self::Compaction(_) => Some("compaction"),
      Self::Subtask(_) => Some("subtask"),
      Self::Retry(_) => Some("retry"),
      Self::StepStart(_) => Some("step-start"),
      Self::StepFinish(_) => Some("step-finish"),
      Self::Tool(_) => Some("tool"),
      Self::Unknown(item) => item.native_type.as_deref(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PartIdentity {
  #[serde(default)]
  pub id: Option<String>,
  #[serde(default, rename = "sessionID")]
  pub session_id: Option<String>,
  #[serde(default, rename = "messageID")]
  pub message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SnapshotPart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  #[serde(default)]
  pub snapshot: Option<String>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PatchPart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  #[serde(default)]
  pub hash: Option<String>,
  #[serde(default)]
  pub files: Vec<String>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextPart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  pub text: String,
  #[serde(default)]
  pub synthetic: Option<bool>,
  #[serde(default)]
  pub ignored: Option<bool>,
  #[serde(default)]
  pub time: Option<PartTime>,
  #[serde(default)]
  pub metadata: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReasoningPart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  pub text: String,
  #[serde(default)]
  pub time: Option<PartTime>,
  #[serde(default)]
  pub metadata: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FilePart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  #[serde(default)]
  pub mime: Option<String>,
  #[serde(default)]
  pub filename: Option<String>,
  #[serde(default)]
  pub url: Option<String>,
  #[serde(default)]
  pub source: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AgentPart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  #[serde(default)]
  pub name: Option<String>,
  #[serde(default)]
  pub source: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CompactionPart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  #[serde(default)]
  pub auto: Option<bool>,
  #[serde(default)]
  pub overflow: Option<bool>,
  #[serde(default)]
  pub tail_start_id: Option<String>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SubtaskPart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  #[serde(default)]
  pub prompt: Option<String>,
  #[serde(default)]
  pub description: Option<String>,
  #[serde(default)]
  pub agent: Option<String>,
  #[serde(default)]
  pub model: Option<MessageModel>,
  #[serde(default)]
  pub command: Option<String>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RetryPart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  #[serde(default)]
  pub attempt: Option<u64>,
  #[serde(default)]
  pub error: Option<Value>,
  #[serde(default)]
  pub time: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StepStartPart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  #[serde(default)]
  pub snapshot: Option<String>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StepFinishPart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  #[serde(default)]
  pub reason: Option<String>,
  #[serde(default)]
  pub snapshot: Option<String>,
  #[serde(default)]
  pub cost: Option<f64>,
  #[serde(default)]
  pub tokens: Option<TokenUsage>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolPart {
  #[serde(flatten)]
  pub identity: PartIdentity,
  #[serde(default, rename = "callID")]
  pub call_id: Option<String>,
  #[serde(default)]
  pub tool: Option<String>,
  pub state: ToolState,
  #[serde(default)]
  pub metadata: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PartTime {
  #[serde(default)]
  pub start: Option<u64>,
  #[serde(default)]
  pub end: Option<u64>,
  #[serde(default)]
  pub created: Option<u64>,
  #[serde(default)]
  pub compacted: Option<u64>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

/// A lossless view of the nested state in a V1 tool part.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolState {
  item: ToolStateItem,
  native: Value,
}

impl ToolState {
  pub fn native_status(&self) -> Option<&str> {
    self.item.native_status()
  }

  pub fn item(&self) -> &ToolStateItem {
    &self.item
  }

  pub fn into_item(self) -> ToolStateItem {
    self.item
  }

  pub fn native(&self) -> &Value {
    &self.native
  }

  pub fn into_native(self) -> Value {
    self.native
  }

  pub fn into_parts(self) -> (ToolStateItem, Value) {
    (self.item, self.native)
  }
}

impl<'de> Deserialize<'de> for ToolState {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let native = Value::deserialize(deserializer)?;
    let native_status = string_field(&native, "status");
    let item = decode_tool_state(native_status, native.clone());
    Ok(Self { item, native })
  }
}

impl Serialize for ToolState {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    self.native.serialize(serializer)
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolStateItem {
  Pending(PendingToolState),
  Running(RunningToolState),
  Completed(CompletedToolState),
  Error(ErrorToolState),
  Unknown(UnknownItem),
}

impl ToolStateItem {
  pub fn native_status(&self) -> Option<&str> {
    match self {
      Self::Pending(_) => Some("pending"),
      Self::Running(_) => Some("running"),
      Self::Completed(_) => Some("completed"),
      Self::Error(_) => Some("error"),
      Self::Unknown(item) => item.native_type.as_deref(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PendingToolState {
  #[serde(default)]
  pub input: Option<Value>,
  #[serde(default)]
  pub raw: Option<String>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RunningToolState {
  #[serde(default)]
  pub input: Option<Value>,
  #[serde(default)]
  pub title: Option<String>,
  #[serde(default)]
  pub metadata: Option<Value>,
  #[serde(default)]
  pub time: Option<PartTime>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CompletedToolState {
  #[serde(default)]
  pub input: Option<Value>,
  #[serde(default)]
  pub output: Option<Value>,
  #[serde(default)]
  pub title: Option<String>,
  #[serde(default)]
  pub metadata: Option<Value>,
  #[serde(default)]
  pub time: Option<PartTime>,
  #[serde(default)]
  pub attachments: Option<Vec<Value>>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ErrorToolState {
  #[serde(default)]
  pub input: Option<Value>,
  #[serde(default)]
  pub raw: Option<String>,
  #[serde(default)]
  pub error: Option<String>,
  #[serde(default)]
  pub metadata: Option<Value>,
  #[serde(default)]
  pub time: Option<PartTime>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

fn decode_message(native_role: Option<String>, native: Value) -> MessageItem {
  match native_role.as_deref() {
    Some("user") => decode_known(native_role, native, |message| MessageItem::User(Box::new(message))),
    Some("assistant") => decode_known(native_role, native, |message| MessageItem::Assistant(Box::new(message))),
    _ => MessageItem::Unknown(UnknownItem {
      native_type: native_role,
      native,
      parse_error: None,
    }),
  }
}

fn decode_part(native_type: Option<String>, native: Value) -> PartItem {
  match native_type.as_deref() {
    Some("snapshot") => decode_part_known(native_type, native, PartItem::Snapshot),
    Some("patch") => decode_part_known(native_type, native, PartItem::Patch),
    Some("text") => decode_part_known(native_type, native, PartItem::Text),
    Some("reasoning") => decode_part_known(native_type, native, PartItem::Reasoning),
    Some("file") => decode_part_known(native_type, native, PartItem::File),
    Some("agent") => decode_part_known(native_type, native, PartItem::Agent),
    Some("compaction") => decode_part_known(native_type, native, PartItem::Compaction),
    Some("subtask") => decode_part_known(native_type, native, PartItem::Subtask),
    Some("retry") => decode_part_known(native_type, native, PartItem::Retry),
    Some("step-start") => decode_part_known(native_type, native, PartItem::StepStart),
    Some("step-finish") => decode_part_known(native_type, native, PartItem::StepFinish),
    Some("tool") => decode_part_known(native_type, native, PartItem::Tool),
    _ => PartItem::Unknown(UnknownItem {
      native_type,
      native,
      parse_error: None,
    }),
  }
}

fn decode_tool_state(native_status: Option<String>, native: Value) -> ToolStateItem {
  match native_status.as_deref() {
    Some("pending") => decode_state_known(native_status, native, ToolStateItem::Pending),
    Some("running") => decode_state_known(native_status, native, ToolStateItem::Running),
    Some("completed") => decode_state_known(native_status, native, ToolStateItem::Completed),
    Some("error") => decode_state_known(native_status, native, ToolStateItem::Error),
    _ => ToolStateItem::Unknown(UnknownItem {
      native_type: native_status,
      native,
      parse_error: None,
    }),
  }
}

fn decode_known<T>(native_type: Option<String>, native: Value, wrap: impl FnOnce(T) -> MessageItem) -> MessageItem
where
  T: DeserializeOwned,
{
  match serde_json::from_value(native.clone()) {
    Ok(item) => wrap(item),
    Err(error) => MessageItem::Unknown(UnknownItem {
      native_type,
      native,
      parse_error: Some(error.to_string()),
    }),
  }
}

fn decode_part_known<T>(native_type: Option<String>, native: Value, wrap: impl FnOnce(T) -> PartItem) -> PartItem
where
  T: DeserializeOwned,
{
  match serde_json::from_value(native.clone()) {
    Ok(item) => wrap(item),
    Err(error) => PartItem::Unknown(UnknownItem {
      native_type,
      native,
      parse_error: Some(error.to_string()),
    }),
  }
}

fn decode_state_known<T>(
  native_type: Option<String>,
  native: Value,
  wrap: impl FnOnce(T) -> ToolStateItem,
) -> ToolStateItem
where
  T: DeserializeOwned,
{
  match serde_json::from_value(native.clone()) {
    Ok(item) => wrap(item),
    Err(error) => ToolStateItem::Unknown(UnknownItem {
      native_type,
      native,
      parse_error: Some(error.to_string()),
    }),
  }
}
