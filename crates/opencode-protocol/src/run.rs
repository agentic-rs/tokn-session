//! JSONL envelopes emitted by `opencode run --format json`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::v1::{PartData, PartItem, ReasoningPart, StepFinishPart, StepStartPart, TextPart, ToolPart};
use crate::{ExtraFields, UnknownItem, string_field};

/// One JSONL record emitted by `opencode run --format json`.
#[derive(Debug, Clone, PartialEq)]
pub struct RunLine {
  session_id: Option<String>,
  timestamp: Option<i64>,
  event: RunEvent,
  native: Value,
}

impl RunLine {
  pub fn session_id(&self) -> Option<&str> {
    self.session_id.as_deref()
  }

  pub fn timestamp(&self) -> Option<i64> {
    self.timestamp
  }

  pub fn event(&self) -> &RunEvent {
    &self.event
  }

  pub fn into_event(self) -> RunEvent {
    self.event
  }

  pub fn native(&self) -> &Value {
    &self.native
  }

  pub fn into_native(self) -> Value {
    self.native
  }

  pub fn into_parts(self) -> (RunEvent, Value) {
    (self.event, self.native)
  }
}

impl<'de> Deserialize<'de> for RunLine {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let native = Value::deserialize(deserializer)?;
    let session_id = string_field(&native, "sessionID");
    let timestamp = native.get("timestamp").and_then(Value::as_i64);
    let native_type = string_field(&native, "type");
    let event = decode_run_event(native_type, native.clone());

    Ok(Self {
      session_id,
      timestamp,
      event,
      native,
    })
  }
}

impl Serialize for RunLine {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    self.native.serialize(serializer)
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunEvent {
  Text(TextPart),
  Reasoning(ReasoningPart),
  ToolUse(ToolPart),
  StepStart(StepStartPart),
  StepFinish(StepFinishPart),
  Error(RunError),
  Unknown(UnknownItem),
}

impl RunEvent {
  pub fn native_type(&self) -> Option<&str> {
    match self {
      Self::Text(_) => Some("text"),
      Self::Reasoning(_) => Some("reasoning"),
      Self::ToolUse(_) => Some("tool_use"),
      Self::StepStart(_) => Some("step_start"),
      Self::StepFinish(_) => Some("step_finish"),
      Self::Error(_) => Some("error"),
      Self::Unknown(item) => item.native_type.as_deref(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunError {
  pub error: Value,
  #[serde(flatten)]
  pub extra: ExtraFields,
}

fn decode_run_event(native_type: Option<String>, native: Value) -> RunEvent {
  match native_type.as_deref() {
    Some("text" | "reasoning" | "tool_use" | "step_start" | "step_finish") => {
      decode_part_event(native_type.expect("matched event type"), native)
    }
    Some("error") => match serde_json::from_value::<RunError>(native.clone()) {
      Ok(error) => RunEvent::Error(error),
      Err(error) => unknown(native_type, native, Some(error.to_string())),
    },
    _ => unknown(native_type, native, None),
  }
}

fn decode_part_event(native_type: String, native: Value) -> RunEvent {
  let Some(part_native) = native.get("part").cloned() else {
    return unknown(
      Some(native_type),
      native,
      Some("known run event is missing `part`".to_string()),
    );
  };

  let part = match serde_json::from_value::<PartData>(part_native) {
    Ok(part) => part,
    Err(error) => {
      return unknown(Some(native_type), native, Some(error.to_string()));
    }
  };

  let item = part.into_item();
  match (native_type.as_str(), item) {
    ("text", PartItem::Text(part)) => RunEvent::Text(part),
    ("reasoning", PartItem::Reasoning(part)) => RunEvent::Reasoning(part),
    ("tool_use", PartItem::Tool(part)) => RunEvent::ToolUse(part),
    ("step_start", PartItem::StepStart(part)) => RunEvent::StepStart(part),
    ("step_finish", PartItem::StepFinish(part)) => RunEvent::StepFinish(part),
    (_, PartItem::Unknown(item)) => {
      let error = item
        .parse_error
        .unwrap_or_else(|| format!("unexpected embedded part type {:?}", item.native_type));
      unknown(Some(native_type), native, Some(error))
    }
    (_, item) => unknown(
      Some(native_type.clone()),
      native,
      Some(format!(
        "run event `{native_type}` contains part type {:?}",
        item.native_type()
      )),
    ),
  }
}

fn unknown(native_type: Option<String>, native: Value, parse_error: Option<String>) -> RunEvent {
  RunEvent::Unknown(UnknownItem {
    native_type,
    native,
    parse_error,
  })
}
