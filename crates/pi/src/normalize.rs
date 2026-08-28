use serde_json::Value;

use crate::event::{PiContentBlock, PiMessage, PiMessageItem, PiSessionItem, PiSessionLine, PiUserContent};
use tokn_session_core::{
  AgentEvent, ErrorEvent, MessageDelivery, MessageEvent, Phase, Provider, ProviderChanged, ReasoningEvent, Role,
  SessionStarted, ToolCallEvent, UnknownEvent, UsageKind, tool_kind_for_name, tool_summary_for_input,
};

pub struct PiNormalizer {
  session_id: Option<String>,
}

impl PiNormalizer {
  pub fn new() -> Self {
    Self { session_id: None }
  }

  pub fn normalize(&mut self, line: PiSessionLine) -> Vec<AgentEvent> {
    let line_timestamp = line.timestamp().map(str::to_string);
    let (event, native) = line.into_parts();
    match event {
      PiSessionItem::Session(event) => {
        let Some(session_id) = event.id else {
          return vec![unknown_event(
            self.session_id.clone(),
            Some("session".to_string()),
            Some(native),
            line_timestamp,
          )];
        };
        self.session_id = Some(session_id.clone());
        vec![AgentEvent::SessionStarted(SessionStarted {
          provider: Provider::Pi,
          session_id,
          cwd: event.cwd,
          timestamp: event.timestamp.or(line_timestamp),
        })]
      }
      PiSessionItem::ModelChange(event) => vec![AgentEvent::ProviderChanged(ProviderChanged {
        provider: Provider::Pi,
        session_id: self.session_id.clone(),
        native_id: event.id,
        native_parent_id: event.parent_id,
        model_provider: event.provider,
        model_id: event.model_id,
        thinking_level: None,
        timestamp: event.timestamp.or(line_timestamp),
      })],
      PiSessionItem::ThinkingLevelChange(event) => {
        vec![AgentEvent::ProviderChanged(ProviderChanged {
          provider: Provider::Pi,
          session_id: self.session_id.clone(),
          native_id: event.id,
          native_parent_id: event.parent_id,
          model_provider: None,
          model_id: None,
          thinking_level: event.thinking_level,
          timestamp: event.timestamp.or(line_timestamp),
        })]
      }
      PiSessionItem::Message(event) => normalize_message(self.session_id.clone(), event, native, line_timestamp),
      PiSessionItem::Error(event) => vec![AgentEvent::Error(ErrorEvent {
        provider: Provider::Pi,
        session_id: self.session_id.clone(),
        message: event
          .message
          .or_else(|| event.error.map(|value| value.to_string()))
          .unwrap_or_else(|| "unknown pi error".to_string()),
        timestamp: event.timestamp.or(line_timestamp),
      })],
      PiSessionItem::Compaction(_)
      | PiSessionItem::BranchSummary(_)
      | PiSessionItem::Custom(_)
      | PiSessionItem::CustomMessage(_)
      | PiSessionItem::Label(_)
      | PiSessionItem::SessionInfo(_)
      | PiSessionItem::Leaf(_)
      | PiSessionItem::ActiveToolsChange(_) => {
        crate::records::normalize(self.session_id.clone(), native, line_timestamp)
      }
      PiSessionItem::Unknown(event) => vec![unknown_event(
        self.session_id.clone(),
        event.native_type,
        Some(event.native),
        line_timestamp,
      )],
    }
  }
}

fn normalize_message(
  session_id: Option<String>,
  event: PiMessageItem,
  native: Value,
  line_timestamp: Option<String>,
) -> Vec<AgentEvent> {
  let meta = PiMessageMeta {
    id: event.id,
    parent_id: event.parent_id,
    timestamp: event.timestamp.or(line_timestamp),
  };

  let Some(message) = event.message else {
    return vec![unknown_event(
      session_id,
      Some("message".to_string()),
      Some(native),
      meta.timestamp,
    )];
  };

  let usage_kind = match &message {
    PiMessage::Assistant(_) => Some(UsageKind::ModelCall),
    PiMessage::ToolResult(_) => Some(UsageKind::OperationTotal),
    _ => None,
  };
  let accounting = usage_kind.and_then(|kind| {
    native["message"]
      .get("usage")
      .filter(|value| !value.is_null())
      .map(|usage| {
        crate::records::usage(
          session_id.clone(),
          meta.id.clone(),
          true,
          kind,
          usage.clone(),
          meta.timestamp.clone(),
        )
      })
  });
  let mut events = match message {
    PiMessage::User(message) => normalize_user_message(session_id, &meta, message),
    PiMessage::Assistant(message) => normalize_assistant_message(session_id, &meta, message),
    PiMessage::ToolResult(message) => normalize_tool_result_message(session_id, &meta, message),
    PiMessage::Unknown(message) => vec![unknown_event(
      session_id,
      message
        .native_type
        .map(|message_type| format!("message.{message_type}"))
        .or_else(|| Some("message".to_string())),
      Some(message.native),
      meta.timestamp,
    )],
  };
  if let Some(accounting) = accounting {
    // A usage-only assistant record is still useful; don't manufacture unknown
    // content for a valid empty response. Unsupported content remains visible.
    if matches!(accounting, AgentEvent::Usage(_)) && native["message"]["content"] == serde_json::json!([]) {
      events.retain(|event| !matches!(event, AgentEvent::Unknown(event) if event.native.is_none()));
    }
    events.push(accounting);
  }
  events
}

struct PiMessageMeta {
  id: Option<String>,
  parent_id: Option<String>,
  timestamp: Option<String>,
}

fn normalize_user_message(
  session_id: Option<String>,
  meta: &PiMessageMeta,
  message: crate::event::PiUserMessage,
) -> Vec<AgentEvent> {
  let timestamp = meta
    .timestamp
    .clone()
    .or_else(|| message.timestamp.map(|value| value.to_string()));
  let mut events = Vec::new();

  match message.content {
    PiUserContent::Text(text) => events.push(message_event(
      session_id.clone(),
      meta,
      Role::User,
      text,
      timestamp.clone(),
    )),
    PiUserContent::Blocks(blocks) => {
      for block in blocks {
        match block {
          PiContentBlock::Text(content) => {
            let native = json_value(&content);
            if let Some(text) = content.text.filter(|text| !text.is_empty()) {
              events.push(message_event(
                session_id.clone(),
                meta,
                Role::User,
                text,
                timestamp.clone(),
              ));
            } else {
              events.push(unknown_event(
                session_id.clone(),
                Some("message.content.text".to_string()),
                Some(native),
                timestamp.clone(),
              ));
            }
          }
          PiContentBlock::Image(content) => {
            events.push(unknown_event(
              session_id.clone(),
              Some("message.content.image".to_string()),
              Some(serde_json::json!({
                "type": "image",
                "mime_type": content.mime_type,
              })),
              timestamp.clone(),
            ));
          }
          PiContentBlock::Unknown(content) => events.push(unknown_event(
            session_id.clone(),
            prefixed_type("message.content", content.native_type.as_deref()),
            Some(content.native),
            timestamp.clone(),
          )),
          content => {
            let native_type = prefixed_type("message.content", content.native_type());
            events.push(unknown_event(
              session_id.clone(),
              native_type,
              Some(json_value(content)),
              timestamp.clone(),
            ));
          }
        }
      }
    }
    PiUserContent::Missing => {}
    PiUserContent::Unknown(content) => events.push(unknown_event(
      session_id.clone(),
      Some("message.content".to_string()),
      Some(content),
      timestamp.clone(),
    )),
  }

  ensure_message_events(events, session_id, timestamp)
}

fn normalize_assistant_message(
  session_id: Option<String>,
  meta: &PiMessageMeta,
  message: crate::event::PiAssistantMessage,
) -> Vec<AgentEvent> {
  let timestamp = meta
    .timestamp
    .clone()
    .or_else(|| message.timestamp.map(|value| value.to_string()));
  let mut events = Vec::new();

  for block in message.content {
    match block {
      PiContentBlock::Text(content) => {
        let native = json_value(&content);
        if let Some(text) = content.text.filter(|text| !text.is_empty()) {
          events.push(message_event(
            session_id.clone(),
            meta,
            Role::Assistant,
            text,
            timestamp.clone(),
          ));
        } else {
          events.push(unknown_event(
            session_id.clone(),
            Some("message.content.text".to_string()),
            Some(native),
            timestamp.clone(),
          ));
        }
      }
      PiContentBlock::Thinking(content) => {
        if content.thinking.is_some() || content.thinking_signature.is_some() {
          events.push(AgentEvent::Reasoning(ReasoningEvent {
            provenance: None,
            provider: Provider::Pi,
            session_id: session_id.clone(),
            message_id: meta.id.clone(),
            parent_id: meta.parent_id.clone(),
            phase: Phase::Finished,
            text: content.thinking.and_then(present_text),
            summary: None,
            encrypted_content: None,
            signature: content.thinking_signature,
            timestamp: timestamp.clone(),
          }));
        }
      }
      PiContentBlock::ToolCall(content) => {
        let native = json_value(&content);
        let Some(name) = content.name else {
          events.push(unknown_event(
            session_id.clone(),
            Some("message.content.toolCall".to_string()),
            Some(native),
            timestamp.clone(),
          ));
          continue;
        };
        events.push(AgentEvent::ToolCall(ToolCallEvent {
          provider: Provider::Pi,
          session_id: session_id.clone(),
          message_id: meta.id.clone(),
          parent_id: meta.parent_id.clone(),
          tool_call_id: content.id,
          tool_name: Some(name.clone()),
          tool_kind: tool_kind_for_name(&name),
          summary: tool_summary_for_input(&name, &content.arguments),
          phase: Phase::Finished,
          input: Some(content.arguments),
          output: None,
          is_error: None,
          timestamp: timestamp.clone(),
        }));
      }
      PiContentBlock::Image(content) => events.push(unknown_event(
        session_id.clone(),
        Some("message.content.image".to_string()),
        Some(serde_json::json!({
          "type": "image",
          "mime_type": content.mime_type,
        })),
        timestamp.clone(),
      )),
      PiContentBlock::Unknown(content) => events.push(unknown_event(
        session_id.clone(),
        prefixed_type("message.content", content.native_type.as_deref()),
        Some(content.native),
        timestamp.clone(),
      )),
    }
  }

  ensure_message_events(events, session_id, timestamp)
}

fn normalize_tool_result_message(
  session_id: Option<String>,
  meta: &PiMessageMeta,
  message: crate::event::PiToolResultMessage,
) -> Vec<AgentEvent> {
  let timestamp = meta
    .timestamp
    .clone()
    .or_else(|| message.timestamp.map(|value| value.to_string()));
  let output = message
    .details
    .unwrap_or_else(|| Value::Array(message.content.into_iter().map(content_block_to_value).collect()));
  let tool_name = message.tool_name;

  vec![AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Pi,
    session_id,
    message_id: meta.id.clone(),
    parent_id: meta.parent_id.clone(),
    tool_call_id: message.tool_call_id,
    tool_name: tool_name.clone(),
    tool_kind: tool_name
      .as_deref()
      .map(tool_kind_for_name)
      .unwrap_or(tokn_session_core::ToolKind::Unknown),
    summary: tool_name
      .as_deref()
      .and_then(|tool_name| tool_summary_for_input(tool_name, &output)),
    phase: Phase::Finished,
    input: None,
    output: Some(output),
    is_error: message.is_error,
    timestamp,
  })]
}

fn message_event(
  session_id: Option<String>,
  meta: &PiMessageMeta,
  role: Role,
  text: String,
  timestamp: Option<String>,
) -> AgentEvent {
  AgentEvent::Message(MessageEvent {
    provenance: None,
    provider: Provider::Pi,
    session_id,
    message_id: meta.id.clone(),
    parent_id: meta.parent_id.clone(),
    role,
    delivery: match role {
      Role::Assistant => MessageDelivery::Final,
      _ => MessageDelivery::Unspecified,
    },
    phase: Phase::Finished,
    text,
    timestamp,
  })
}

fn ensure_message_events(
  events: Vec<AgentEvent>,
  session_id: Option<String>,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  if events.is_empty() {
    return vec![unknown_event(session_id, Some("message".to_string()), None, timestamp)];
  }
  events
}

fn content_block_to_value(block: PiContentBlock) -> Value {
  match block {
    PiContentBlock::Text(content) => serde_json::json!({
        "type": "text",
        "text": content.text,
    }),
    PiContentBlock::Image(content) => serde_json::json!({
        "type": "image",
        "data": content.data,
        "mime_type": content.mime_type,
    }),
    PiContentBlock::Unknown(content) => content.native,
    content => json_value(content),
  }
}

fn unknown_event(
  session_id: Option<String>,
  native_type: Option<String>,
  native: Option<Value>,
  timestamp: Option<String>,
) -> AgentEvent {
  AgentEvent::Unknown(UnknownEvent {
    provider: Provider::Pi,
    session_id,
    native_type,
    native,
    timestamp,
  })
}

fn present_text(text: String) -> Option<String> {
  (!text.is_empty()).then_some(text)
}

fn prefixed_type(prefix: &str, native_type: Option<&str>) -> Option<String> {
  Some(match native_type {
    Some(native_type) => format!("{prefix}.{native_type}"),
    None => prefix.to_string(),
  })
}

fn json_value(value: impl serde::Serialize) -> Value {
  serde_json::to_value(value).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
  use super::*;
  use tokn_session_core::{ToolKind, ToolSummary};

  #[test]
  fn normalizes_basic_fixture_without_stopping_at_unknown_roles() {
    let events = normalize_fixture(include_str!("../fixtures/basic_session.jsonl"));

    assert_eq!(events.len(), 11);
    assert!(matches!(&events[0], AgentEvent::SessionStarted(event) if event.session_id == "pi-session"));
    assert!(
      matches!(&events[1], AgentEvent::ProviderChanged(event) if event.model_provider.as_deref() == Some("openai") && event.model_id.as_deref() == Some("gpt-5"))
    );
    assert!(
      matches!(&events[2], AgentEvent::ProviderChanged(event) if event.thinking_level.as_deref() == Some("high"))
    );
    assert!(
      matches!(&events[3], AgentEvent::Message(event) if matches!(event.role, Role::User) && event.text == "inspect the project")
    );
    assert!(
      matches!(&events[4], AgentEvent::Reasoning(event) if event.text.as_deref() == Some("checking files") && event.signature.as_deref() == Some("sig-1"))
    );
    assert!(
      matches!(&events[5], AgentEvent::ToolCall(event) if event.tool_call_id.as_deref() == Some("call-1") && matches!(event.tool_kind, ToolKind::FileRead))
    );
    assert!(
      matches!(&events[6], AgentEvent::Message(event) if matches!(event.role, Role::Assistant) && matches!(event.delivery, MessageDelivery::Final) && event.text == "done")
    );
    assert!(
      matches!(&events[7], AgentEvent::ToolCall(event) if event.tool_call_id.as_deref() == Some("call-1") && event.is_error == Some(false))
    );
    assert!(
      matches!(&events[8], AgentEvent::Unknown(event) if event.native_type.as_deref() == Some("message.bashExecution"))
    );
    assert!(matches!(&events[9], AgentEvent::Unknown(event) if event.native_type.as_deref() == Some("compaction")));
    assert!(matches!(&events[10], AgentEvent::Unknown(event) if event.native_type.as_deref() == Some("future_entry")));

    let AgentEvent::ToolCall(tool) = &events[5] else {
      panic!("expected tool call");
    };
    assert!(matches!(
      tool.summary,
      Some(ToolSummary::FileRead {
        path: Some(ref path)
      }) if path == "README.md"
    ));
  }

  #[test]
  fn keeps_unknown_content_blocks_visible() {
    let events = normalize_fixture(
      r#"{"type":"session","id":"pi-session"}
{"type":"message","id":"assistant-1","message":{"role":"assistant","content":[{"type":"future_block","answer":42}]}}"#,
    );

    let AgentEvent::Unknown(event) = &events[1] else {
      panic!("expected unknown content");
    };
    assert_eq!(event.native_type.as_deref(), Some("message.content.future_block"));
    assert_eq!(
      event.native.as_ref().and_then(|value| value.get("answer")),
      Some(&serde_json::json!(42))
    );
  }

  fn normalize_fixture(input: &str) -> Vec<AgentEvent> {
    let mut normalizer = PiNormalizer::new();
    input
      .lines()
      .filter(|line| !line.trim().is_empty())
      .flat_map(|line| {
        let line: PiSessionLine = serde_json::from_str(line).expect("fixture line should decode");
        normalizer.normalize(line)
      })
      .collect()
  }
}
