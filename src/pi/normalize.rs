use serde_json::Value;

use crate::agent_event::{
    AgentEvent, ErrorEvent, MessageEvent, Phase, Provider, ProviderChanged, ReasoningEvent, Role,
    SessionStarted, ToolCallEvent, UnknownEvent,
};
use crate::pi::event::{PiEvent, PiMessageEvent};

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
    let mut events = Vec::new();
    let role = role_from_pi(&event.message.role);
    let timestamp = event
        .timestamp
        .or_else(|| event.message.timestamp.map(|value| value.to_string()));

    for content in content_items(&event.message.content) {
        let content_type = content
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("text");

        match content_type {
            "text" => {
                if let Some(text) = content.get("text").and_then(Value::as_str) {
                    events.push(AgentEvent::Message(MessageEvent {
                        provider: Provider::Pi,
                        session_id: session_id.clone(),
                        message_id: event.id.clone(),
                        parent_id: event.parent_id.clone(),
                        role,
                        phase: Phase::Finished,
                        text: text.to_string(),
                        timestamp: timestamp.clone(),
                    }));
                }
            }
            "thinking" | "reasoning" => {
                let text = content
                    .get("thinking")
                    .or_else(|| content.get("text"))
                    .and_then(Value::as_str);
                if let Some(text) = text {
                    events.push(AgentEvent::Reasoning(ReasoningEvent {
                        provider: Provider::Pi,
                        session_id: session_id.clone(),
                        message_id: event.id.clone(),
                        parent_id: event.parent_id.clone(),
                        phase: Phase::Finished,
                        text: text.to_string(),
                        timestamp: timestamp.clone(),
                    }));
                }
            }
            "tool_use" | "tool_call" => {
                events.push(AgentEvent::ToolCall(ToolCallEvent {
                    provider: Provider::Pi,
                    session_id: session_id.clone(),
                    message_id: event.id.clone(),
                    parent_id: event.parent_id.clone(),
                    tool_name: content
                        .get("name")
                        .or_else(|| content.get("tool"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    phase: Phase::Finished,
                    input: content
                        .get("input")
                        .cloned()
                        .or_else(|| content.get("args").cloned()),
                    output: None,
                    timestamp: timestamp.clone(),
                }));
            }
            "tool_result" => {
                events.push(AgentEvent::ToolCall(ToolCallEvent {
                    provider: Provider::Pi,
                    session_id: session_id.clone(),
                    message_id: event.id.clone(),
                    parent_id: event.parent_id.clone(),
                    tool_name: None,
                    phase: Phase::Finished,
                    input: None,
                    output: Some(content.clone()),
                    timestamp: timestamp.clone(),
                }));
            }
            _ => events.push(AgentEvent::Unknown(UnknownEvent {
                provider: Provider::Pi,
                session_id: session_id.clone(),
                native_type: Some(format!("message.content.{content_type}")),
                timestamp: timestamp.clone(),
            })),
        }
    }

    if events.is_empty() {
        events.push(AgentEvent::Unknown(UnknownEvent {
            provider: Provider::Pi,
            session_id,
            native_type: Some("message".to_string()),
            timestamp,
        }));
    }

    events
}

fn content_items(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.clone(),
        Value::String(text) => vec![serde_json::json!({ "type": "text", "text": text })],
        Value::Object(_) => vec![value.clone()],
        _ => Vec::new(),
    }
}

fn role_from_pi(role: &str) -> Role {
    match role {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "system" => Role::System,
        "tool" => Role::Tool,
        _ => Role::Unknown,
    }
}
