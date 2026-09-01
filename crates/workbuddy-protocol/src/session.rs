use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

pub type ExtraFields = Map<String, Value>;

/// One JSONL record from a persisted WorkBuddy session history.
///
/// WorkBuddy's record and content enums may grow independently of this crate.
/// The native record is therefore retained beside the typed view so callers
/// can inspect or forward every provider field, including unknown variants.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkBuddySessionLine {
  id: Option<String>,
  parent_id: Option<String>,
  timestamp: Option<u64>,
  session_id: Option<String>,
  cwd: Option<String>,
  item: WorkBuddySessionItem,
  native: Value,
}

impl WorkBuddySessionLine {
  pub fn id(&self) -> Option<&str> {
    self.id.as_deref()
  }

  pub fn parent_id(&self) -> Option<&str> {
    self.parent_id.as_deref()
  }

  pub fn timestamp(&self) -> Option<u64> {
    self.timestamp
  }

  pub fn session_id(&self) -> Option<&str> {
    self.session_id.as_deref()
  }

  pub fn cwd(&self) -> Option<&str> {
    self.cwd.as_deref()
  }

  pub fn item(&self) -> &WorkBuddySessionItem {
    &self.item
  }

  pub fn into_item(self) -> WorkBuddySessionItem {
    self.item
  }

  pub fn native(&self) -> &Value {
    &self.native
  }

  pub fn into_native(self) -> Value {
    self.native
  }

  pub fn into_parts(self) -> (WorkBuddySessionItem, Value) {
    (self.item, self.native)
  }
}

impl<'de> Deserialize<'de> for WorkBuddySessionLine {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let native = Value::deserialize(deserializer)?;
    let id = string_field(&native, "id");
    let parent_id = string_field(&native, "parentId");
    let timestamp = native.get("timestamp").and_then(Value::as_u64);
    let session_id = string_field(&native, "sessionId");
    let cwd = string_field(&native, "cwd");
    let native_type = string_field(&native, "type");
    let item = decode_session_item(native_type, native.clone());

    Ok(Self {
      id,
      parent_id,
      timestamp,
      session_id,
      cwd,
      item,
      native,
    })
  }
}

impl Serialize for WorkBuddySessionLine {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    self.native.serialize(serializer)
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkBuddySessionItem {
  Message(MessageItem),
  FunctionCall(FunctionCallItem),
  FunctionCallResult(FunctionCallResultItem),
  Reasoning(ReasoningItem),
  FileHistorySnapshot(FileHistorySnapshotItem),
  AiTitle(AiTitleItem),
  Unknown(UnknownItem),
}

impl WorkBuddySessionItem {
  pub fn native_type(&self) -> Option<&str> {
    match self {
      Self::Message(_) => Some("message"),
      Self::FunctionCall(_) => Some("function_call"),
      Self::FunctionCallResult(_) => Some("function_call_result"),
      Self::Reasoning(_) => Some("reasoning"),
      Self::FileHistorySnapshot(_) => Some("file-history-snapshot"),
      Self::AiTitle(_) => Some("ai-title"),
      Self::Unknown(item) => item.native_type.as_deref(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MessageItem {
  #[serde(default)]
  pub role: Option<String>,
  #[serde(default)]
  pub status: Option<String>,
  #[serde(default)]
  pub content: Vec<ContentBlock>,
  #[serde(default)]
  pub provider_data: Option<Value>,
  #[serde(default)]
  pub message: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCallItem {
  #[serde(default)]
  pub call_id: Option<String>,
  #[serde(default)]
  pub name: Option<String>,
  /// The fixtures currently store JSON-encoded arguments in a string. This is
  /// intentionally a JSON value so future native object arguments still decode.
  #[serde(default)]
  pub arguments: Option<Value>,
  #[serde(default)]
  pub provider_data: Option<Value>,
  #[serde(default)]
  pub message: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCallResultItem {
  #[serde(default)]
  pub call_id: Option<String>,
  #[serde(default)]
  pub name: Option<String>,
  #[serde(default)]
  pub status: Option<String>,
  #[serde(default)]
  pub output: Option<ContentBlock>,
  #[serde(default)]
  pub provider_data: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningItem {
  #[serde(default)]
  pub content: Vec<ContentBlock>,
  #[serde(default)]
  pub raw_content: Vec<ContentBlock>,
  #[serde(default)]
  pub provider_data: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileHistorySnapshotItem {
  #[serde(default)]
  pub is_snapshot_update: Option<bool>,
  #[serde(default)]
  pub snapshot: Option<FileHistorySnapshot>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileHistorySnapshot {
  #[serde(default)]
  pub message_id: Option<String>,
  #[serde(default)]
  pub tracked_file_backups: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiTitleItem {
  #[serde(default)]
  pub ai_title: Option<String>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

/// A content object nested in a message, reasoning record, or tool result.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
  InputText(TextContent),
  OutputText(TextContent),
  ReasoningText(TextContent),
  Text(TextContent),
  Unknown(UnknownItem),
}

impl ContentBlock {
  pub fn native_type(&self) -> Option<&str> {
    match self {
      Self::InputText(_) => Some("input_text"),
      Self::OutputText(_) => Some("output_text"),
      Self::ReasoningText(_) => Some("reasoning_text"),
      Self::Text(_) => Some("text"),
      Self::Unknown(item) => item.native_type.as_deref(),
    }
  }

  pub fn text(&self) -> Option<&str> {
    match self {
      Self::InputText(item) | Self::OutputText(item) | Self::ReasoningText(item) | Self::Text(item) => {
        item.text.as_deref()
      }
      Self::Unknown(_) => None,
    }
  }
}

impl<'de> Deserialize<'de> for ContentBlock {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let native = Value::deserialize(deserializer)?;
    Ok(decode_content_block(native))
  }
}

impl Serialize for ContentBlock {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    match self {
      Self::InputText(item) | Self::OutputText(item) | Self::ReasoningText(item) | Self::Text(item) => {
        item.serialize(serializer)
      }
      Self::Unknown(item) => item.native.serialize(serializer),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TextContent {
  #[serde(default)]
  pub text: Option<String>,
  #[serde(default)]
  pub provider_data: Option<Value>,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnknownItem {
  pub native_type: Option<String>,
  pub native: Value,
  pub parse_error: Option<String>,
}

fn decode_session_item(native_type: Option<String>, native: Value) -> WorkBuddySessionItem {
  match native_type.as_deref() {
    Some("message") => decode_item(native_type, native, WorkBuddySessionItem::Message),
    Some("function_call") => decode_item(native_type, native, WorkBuddySessionItem::FunctionCall),
    Some("function_call_result") => decode_item(native_type, native, WorkBuddySessionItem::FunctionCallResult),
    Some("reasoning") => decode_item(native_type, native, WorkBuddySessionItem::Reasoning),
    Some("file-history-snapshot") => decode_item(native_type, native, WorkBuddySessionItem::FileHistorySnapshot),
    Some("ai-title") => decode_item(native_type, native, WorkBuddySessionItem::AiTitle),
    _ => WorkBuddySessionItem::Unknown(UnknownItem {
      native_type,
      native,
      parse_error: None,
    }),
  }
}

fn decode_content_block(native: Value) -> ContentBlock {
  let native_type = string_field(&native, "type");
  match native_type.as_deref() {
    Some("input_text") => decode_content(native_type, native, ContentBlock::InputText),
    Some("output_text") => decode_content(native_type, native, ContentBlock::OutputText),
    Some("reasoning_text") => decode_content(native_type, native, ContentBlock::ReasoningText),
    Some("text") => decode_content(native_type, native, ContentBlock::Text),
    _ => ContentBlock::Unknown(UnknownItem {
      native_type,
      native,
      parse_error: None,
    }),
  }
}

fn decode_item<T>(
  native_type: Option<String>,
  native: Value,
  wrap: impl FnOnce(T) -> WorkBuddySessionItem,
) -> WorkBuddySessionItem
where
  T: DeserializeOwned,
{
  match serde_json::from_value(native.clone()) {
    Ok(item) => wrap(item),
    Err(error) => WorkBuddySessionItem::Unknown(UnknownItem {
      native_type,
      native,
      parse_error: Some(error.to_string()),
    }),
  }
}

fn decode_content<T>(native_type: Option<String>, native: Value, wrap: impl FnOnce(T) -> ContentBlock) -> ContentBlock
where
  T: DeserializeOwned,
{
  match serde_json::from_value(native.clone()) {
    Ok(content) => wrap(content),
    Err(error) => ContentBlock::Unknown(UnknownItem {
      native_type,
      native,
      parse_error: Some(error.to_string()),
    }),
  }
}

fn string_field(value: &Value, field: &str) -> Option<String> {
  value.get(field).and_then(Value::as_str).map(str::to_string)
}
