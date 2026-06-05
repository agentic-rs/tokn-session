use codex_protocol::protocol::RolloutItem;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug)]
pub enum CodexEvent {
  RolloutItem(RolloutItem),
  Unknown(Value),
}

#[derive(Debug)]
pub struct CodexLine {
  pub timestamp: Option<String>,
  pub event: CodexEvent,
}

impl<'de> Deserialize<'de> for CodexLine {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: serde::Deserializer<'de>,
  {
    let value = Value::deserialize(deserializer)?;
    let timestamp = value.get("timestamp").and_then(Value::as_str).map(str::to_string);

    let event = match serde_json::from_value::<RolloutItem>(value.clone()) {
      Ok(event) => CodexEvent::RolloutItem(event),
      Err(_) => CodexEvent::Unknown(value),
    };

    Ok(Self { timestamp, event })
  }
}
