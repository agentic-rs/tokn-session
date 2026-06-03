use serde_json::Value;

use crate::event::{
  PiAssistantContentBlock, PiEvent, PiMessage, PiMessageEvent, PiToolResultContentBlock, PiUserContent,
  PiUserContentBlock,
};
use tokn_agent_core::{
  AgentEvent, ErrorEvent, MessageEvent, Phase, Provider, ProviderChanged, ReasoningEvent, Role, SessionStarted,
  ToolCallEvent, UnknownEvent,
};

pub struct PiNormalizer {
  session_id: Option<String>,
}

impl PiNormalizer {
  pub fn new() -> Self {
    Self { session_id: None }
  }

  pub fn normalize(&mut self, event: PiEvent) -> Vec<AgentEvent> {
    match event {
      PiEvent::Session(event) => {
        self.session_id = Some(event.id.clone());
        vec![AgentEvent::SessionStarted(SessionStarted {
          provider: Provider::Pi,
          session_id: event.id,
          cwd: event.cwd,
          timestamp: event.timestamp,
        })]
      }
      PiEvent::ModelChange(event) => vec![AgentEvent::ProviderChanged(ProviderChanged {
        provider: Provider::Pi,
        session_id: self.session_id.clone(),
        native_id: event.id,
        native_parent_id: event.parent_id,
        model_provider: event.provider,
        model_id: event.model_id,
        thinking_level: None,
        timestamp: event.timestamp,
      })],
      PiEvent::ThinkingLevelChange(event) => {
        let _ = (&event.id, &event.parent_id);
        vec![AgentEvent::ProviderChanged(ProviderChanged {
          provider: Provider::Pi,
          session_id: self.session_id.clone(),
          native_id: event.id,
          native_parent_id: event.parent_id,
          model_provider: None,
          model_id: None,
          thinking_level: event.thinking_level,
          timestamp: event.timestamp,
        })]
      }
      PiEvent::Message(event) => normalize_message(self.session_id.clone(), event),
      PiEvent::Error(event) => vec![AgentEvent::Error(ErrorEvent {
        provider: Provider::Pi,
        session_id: self.session_id.clone(),
        message: event
          .message
          .or_else(|| event.error.map(|value| value.to_string()))
          .unwrap_or_else(|| "unknown pi error".to_string()),
        timestamp: event.timestamp,
      })],
      PiEvent::Unknown(event) => vec![AgentEvent::Unknown(UnknownEvent {
        provider: Provider::Pi,
        session_id: self.session_id.clone(),
        native_type: event.event_type,
        timestamp: event.timestamp,
      })],
    }
  }
}

fn normalize_message(session_id: Option<String>, event: PiMessageEvent) -> Vec<AgentEvent> {
  let meta = PiMessageMeta {
    id: event.id,
    parent_id: event.parent_id,
    timestamp: event.timestamp,
  };

  match event.message {
    PiMessage::User(message) => normalize_user_message(session_id, &meta, message),
    PiMessage::Assistant(message) => normalize_assistant_message(session_id, &meta, message),
    PiMessage::ToolResult(message) => normalize_tool_result_message(session_id, &meta, message),
  }
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
          PiUserContentBlock::Text { text } => events.push(message_event(
            session_id.clone(),
            meta,
            Role::User,
            text,
            timestamp.clone(),
          )),
          PiUserContentBlock::Image { data, mime_type } => {
            let _ = (data, mime_type);
            events.push(unknown_event(
              session_id.clone(),
              Some("message.content.image".to_string()),
              timestamp.clone(),
            ));
          }
          PiUserContentBlock::Unknown(value) => events.push(unknown_event(
            session_id.clone(),
            unknown_content_type("message.content", &value),
            timestamp.clone(),
          )),
        }
      }
    }
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
  let _ = (&message.provider, &message.model);

  for block in message.content {
    match block {
      PiAssistantContentBlock::Text { text } => events.push(message_event(
        session_id.clone(),
        meta,
        Role::Assistant,
        text,
        timestamp.clone(),
      )),
      PiAssistantContentBlock::Thinking {
        thinking,
        thinking_signature,
      } => {
        let _ = thinking_signature;
        events.push(AgentEvent::Reasoning(ReasoningEvent {
          provider: Provider::Pi,
          session_id: session_id.clone(),
          message_id: meta.id.clone(),
          parent_id: meta.parent_id.clone(),
          phase: Phase::Finished,
          text: present_text(thinking),
          summary: None,
          encrypted_content: None,
          signature: thinking_signature,
          timestamp: timestamp.clone(),
        }));
      }
      PiAssistantContentBlock::ToolCall { id, name, arguments } => events.push(AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Pi,
        session_id: session_id.clone(),
        message_id: meta.id.clone(),
        parent_id: meta.parent_id.clone(),
        tool_call_id: Some(id),
        tool_name: Some(name),
        phase: Phase::Finished,
        input: Some(arguments),
        output: None,
        is_error: None,
        timestamp: timestamp.clone(),
      })),
      PiAssistantContentBlock::Unknown(value) => events.push(unknown_event(
        session_id.clone(),
        unknown_content_type("message.content", &value),
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
    .unwrap_or_else(|| Value::Array(message.content.into_iter().map(tool_result_content_to_value).collect()));

  vec![AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Pi,
    session_id,
    message_id: meta.id.clone(),
    parent_id: meta.parent_id.clone(),
    tool_call_id: Some(message.tool_call_id),
    tool_name: Some(message.tool_name),
    phase: Phase::Finished,
    input: None,
    output: Some(output),
    is_error: Some(message.is_error),
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
    provider: Provider::Pi,
    session_id,
    message_id: meta.id.clone(),
    parent_id: meta.parent_id.clone(),
    role,
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
    return vec![unknown_event(session_id, Some("message".to_string()), timestamp)];
  }
  events
}

fn tool_result_content_to_value(block: PiToolResultContentBlock) -> Value {
  match block {
    PiToolResultContentBlock::Text { text } => serde_json::json!({
        "type": "text",
        "text": text,
    }),
    PiToolResultContentBlock::Image { data, mime_type } => serde_json::json!({
        "type": "image",
        "data": data,
        "mime_type": mime_type,
    }),
    PiToolResultContentBlock::Unknown(value) => value,
  }
}

fn unknown_event(session_id: Option<String>, native_type: Option<String>, timestamp: Option<String>) -> AgentEvent {
  AgentEvent::Unknown(UnknownEvent {
    provider: Provider::Pi,
    session_id,
    native_type,
    timestamp,
  })
}

fn present_text(text: String) -> Option<String> {
  (!text.is_empty()).then_some(text)
}

fn unknown_content_type(prefix: &str, value: &Value) -> Option<String> {
  let suffix = value.get("type").and_then(Value::as_str).unwrap_or("unknown");
  Some(format!("{prefix}.{suffix}"))
}
