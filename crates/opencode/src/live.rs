use serde::Deserialize;
use serde_json::Value;
use tokn_session_core::{
  AgentEvent, ErrorEvent, LiveSessionEvent, MessageDelivery, MessageEvent, Phase, Provider, ReasoningEvent, Role,
  UnknownEvent,
};

use crate::event::OpenCodeToolState;
use crate::normalize::{timestamp, tool_event};

pub struct OpenCodeLiveNormalizer;

impl OpenCodeLiveNormalizer {
  pub fn normalize_line(line: &str) -> Result<Vec<LiveSessionEvent>, String> {
    if line.trim().is_empty() {
      return Ok(Vec::new());
    }

    let native: Value = serde_json::from_str(line).map_err(|err| format!("invalid opencode live json: {err}"))?;
    let event: OpenCodeLiveLine =
      serde_json::from_value(native.clone()).map_err(|err| format!("invalid opencode live event: {err}"))?;

    Ok(match event.event_type.as_str() {
      "text" => event
        .part
        .and_then(|part| text_event(event.session_id, event.timestamp, part))
        .map(wrap_agent_event)
        .into_iter()
        .collect(),
      "reasoning" => event
        .part
        .and_then(|part| reasoning_event(event.session_id, event.timestamp, part))
        .map(wrap_agent_event)
        .into_iter()
        .collect(),
      "tool_use" => event
        .part
        .and_then(|part| tool_live_event(event.session_id, event.timestamp, part))
        .map(wrap_agent_event)
        .into_iter()
        .collect(),
      "error" => vec![wrap_agent_event(AgentEvent::Error(ErrorEvent {
        provider: Provider::OpenCode,
        session_id: event.session_id,
        message: error_message(event.error.unwrap_or(native)),
        timestamp: timestamp(event.timestamp),
      }))],
      _ => vec![LiveSessionEvent::Unknown(UnknownEvent {
        provider: Provider::OpenCode,
        session_id: event.session_id,
        native_type: Some(event.event_type),
        native: Some(native),
        timestamp: timestamp(event.timestamp),
      })],
    })
  }
}

#[derive(Debug, Deserialize)]
struct OpenCodeLiveLine {
  #[serde(rename = "type")]
  event_type: String,
  #[serde(rename = "sessionID")]
  session_id: Option<String>,
  timestamp: Option<i64>,
  part: Option<OpenCodeLivePart>,
  error: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum OpenCodeLivePart {
  Text {
    id: Option<String>,
    #[serde(rename = "messageID")]
    message_id: Option<String>,
    text: String,
  },
  Reasoning {
    id: Option<String>,
    #[serde(rename = "messageID")]
    message_id: Option<String>,
    text: String,
  },
  Tool {
    id: Option<String>,
    #[serde(rename = "callID")]
    call_id: Option<String>,
    #[serde(rename = "messageID")]
    message_id: Option<String>,
    tool: Option<String>,
    state: OpenCodeToolState,
  },
  StepStart {},
  StepFinish {},
  #[allow(dead_code)]
  #[serde(untagged)]
  Unknown(Value),
}

fn text_event(session_id: Option<String>, raw_timestamp: Option<i64>, part: OpenCodeLivePart) -> Option<AgentEvent> {
  match part {
    OpenCodeLivePart::Text { id, message_id, text } => Some(AgentEvent::Message(MessageEvent {
      provider: Provider::OpenCode,
      session_id,
      message_id: message_id.or(id),
      parent_id: None,
      role: Role::Assistant,
      delivery: MessageDelivery::Final,
      phase: Phase::Finished,
      text,
      timestamp: timestamp(raw_timestamp),
    })),
    _ => None,
  }
}

fn reasoning_event(
  session_id: Option<String>,
  raw_timestamp: Option<i64>,
  part: OpenCodeLivePart,
) -> Option<AgentEvent> {
  match part {
    OpenCodeLivePart::Reasoning { id, message_id, text } => Some(AgentEvent::Reasoning(ReasoningEvent {
      provider: Provider::OpenCode,
      session_id,
      message_id: message_id.or(id),
      parent_id: None,
      phase: Phase::Finished,
      text: Some(text),
      summary: None,
      encrypted_content: None,
      signature: None,
      timestamp: timestamp(raw_timestamp),
    })),
    _ => None,
  }
}

fn tool_live_event(session_id: Option<String>, timestamp: Option<i64>, part: OpenCodeLivePart) -> Option<AgentEvent> {
  match part {
    OpenCodeLivePart::Tool {
      id,
      call_id,
      message_id,
      tool,
      state,
    } => {
      let session_id = session_id.unwrap_or_default();
      let message_id = message_id.unwrap_or_else(|| "live".to_string());
      Some(tool_event(
        session_id,
        message_id,
        None,
        call_id.or(id),
        tool,
        state,
        timestamp,
      ))
    }
    _ => None,
  }
}

fn wrap_agent_event(event: AgentEvent) -> LiveSessionEvent {
  LiveSessionEvent::Event(event)
}

fn error_message(value: Value) -> String {
  value
    .get("data")
    .and_then(|data| data.get("message"))
    .and_then(Value::as_str)
    .or_else(|| value.get("message").and_then(Value::as_str))
    .or_else(|| value.as_str())
    .map(str::to_string)
    .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
  use tokn_session_core::{AgentEvent, LiveSessionEvent, MessageDelivery, ToolSummary};

  use super::OpenCodeLiveNormalizer;

  #[test]
  fn normalizes_text_line_to_live_agent_event() {
    let events = OpenCodeLiveNormalizer::normalize_line(
      r#"{"type":"text","timestamp":1710000000000,"sessionID":"ses_123","part":{"id":"part_1","sessionID":"ses_123","messageID":"msg_1","type":"text","text":"done","time":{"end":1710000000000}}}"#,
    )
    .unwrap();

    assert_eq!(events.len(), 1);
    let LiveSessionEvent::Event(AgentEvent::Message(message)) = &events[0] else {
      panic!("expected live message event");
    };
    assert_eq!(message.session_id.as_deref(), Some("ses_123"));
    assert_eq!(message.message_id.as_deref(), Some("msg_1"));
    assert!(matches!(message.delivery, MessageDelivery::Final));
    assert_eq!(message.text, "done");
    assert_eq!(message.timestamp.as_deref(), Some("1710000000000"));
  }

  #[test]
  fn normalizes_tool_use_line_to_semantic_tool_event() {
    let events = OpenCodeLiveNormalizer::normalize_line(
      r#"{"type":"tool_use","timestamp":1710000000001,"sessionID":"ses_123","part":{"id":"tool_1","sessionID":"ses_123","messageID":"msg_2","type":"tool","tool":"bash","state":{"status":"completed","input":{"command":"cargo test"},"output":"ok","metadata":{"exit":0}}}}"#,
    )
    .unwrap();

    assert_eq!(events.len(), 1);
    let LiveSessionEvent::Event(AgentEvent::ToolCall(tool)) = &events[0] else {
      panic!("expected live tool event");
    };
    assert_eq!(tool.session_id.as_deref(), Some("ses_123"));
    assert_eq!(tool.message_id.as_deref(), Some("msg_2"));
    assert_eq!(tool.tool_call_id.as_deref(), Some("tool_1"));
    assert_eq!(tool.is_error, Some(false));
    let Some(ToolSummary::Shell { command, exit_code, .. }) = &tool.summary else {
      panic!("expected shell summary");
    };
    assert_eq!(command.as_deref(), Some("cargo test"));
    assert_eq!(*exit_code, Some(0));
  }

  #[test]
  fn preserves_unknown_live_line() {
    let events = OpenCodeLiveNormalizer::normalize_line(
      r#"{"type":"step_start","timestamp":1710000000002,"sessionID":"ses_123","part":{"id":"step_1","type":"step-start"}}"#,
    )
    .unwrap();

    assert_eq!(events.len(), 1);
    let LiveSessionEvent::Unknown(event) = &events[0] else {
      panic!("expected unknown live event");
    };
    assert_eq!(event.session_id.as_deref(), Some("ses_123"));
    assert_eq!(event.native_type.as_deref(), Some("step_start"));
    assert!(event.native.is_some());
  }
}
