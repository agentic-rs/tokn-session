use serde_json::Value;

use crate::event::{
  CodexCompacted, CodexContentItem, CodexEvent, CodexEventMsg, CodexLine, CodexReasoningContent, CodexResponseItem,
  CodexSessionMeta,
};
use tokn_agent_core::{
  AgentEvent, ErrorEvent, MessageEvent, Phase, Provider, ProviderChanged, ReasoningEvent, Role, SessionStarted,
  ToolCallEvent, UnknownEvent,
};

pub struct CodexNormalizer {
  session_id: Option<String>,
}

impl CodexNormalizer {
  pub fn new() -> Self {
    Self { session_id: None }
  }

  pub fn normalize(&mut self, line: CodexLine) -> Vec<AgentEvent> {
    let timestamp = line.timestamp;
    match line.event {
      CodexEvent::SessionMeta(event) => self.normalize_session_meta(event, timestamp),
      CodexEvent::ResponseItem(event) => normalize_response_item(self.session_id.clone(), event, timestamp),
      CodexEvent::EventMsg(event) => normalize_event_msg(self.session_id.clone(), event, timestamp),
      CodexEvent::TurnContext(value) => {
        let _ = value;
        Vec::new()
      }
      CodexEvent::Compacted(event) => normalize_compacted(self.session_id.clone(), event, timestamp),
    }
  }

  fn normalize_session_meta(&mut self, event: CodexSessionMeta, line_timestamp: Option<String>) -> Vec<AgentEvent> {
    self.session_id = Some(event.id.clone());
    let timestamp = event.timestamp.or(line_timestamp);
    let mut events = vec![AgentEvent::SessionStarted(SessionStarted {
      provider: Provider::Codex,
      session_id: event.id,
      cwd: event.cwd,
      timestamp: timestamp.clone(),
    })];

    if event.model_provider.is_some() {
      events.push(AgentEvent::ProviderChanged(ProviderChanged {
        provider: Provider::Codex,
        session_id: self.session_id.clone(),
        native_id: None,
        native_parent_id: None,
        model_provider: event.model_provider,
        model_id: None,
        thinking_level: None,
        timestamp,
      }));
    }

    events
  }
}

fn normalize_response_item(
  session_id: Option<String>,
  event: CodexResponseItem,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  match event {
    CodexResponseItem::Message {
      id,
      role,
      content,
      phase: _,
    } => normalize_message(session_id, id, role, content, timestamp),
    CodexResponseItem::Reasoning { id, summary, content } => {
      let text = content
        .unwrap_or_default()
        .into_iter()
        .filter_map(reasoning_content_text)
        .chain(summary.into_iter().filter_map(|summary| summary.text))
        .collect::<Vec<_>>()
        .join("\n");
      if text.is_empty() {
        vec![unknown_event(
          session_id,
          Some("response_item.reasoning".to_string()),
          timestamp,
        )]
      } else {
        vec![AgentEvent::Reasoning(ReasoningEvent {
          provider: Provider::Codex,
          session_id,
          message_id: id,
          parent_id: None,
          phase: Phase::Finished,
          text,
          timestamp,
        })]
      }
    }
    CodexResponseItem::FunctionCall {
      id,
      name,
      namespace,
      arguments,
      call_id,
    } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: id,
      parent_id: None,
      tool_call_id: Some(call_id),
      tool_name: Some(namespace.map_or(name.clone(), |namespace| format!("{namespace}.{name}"))),
      phase: Phase::Finished,
      input: Some(parse_json_string_or_text(arguments)),
      output: None,
      is_error: None,
      timestamp,
    })],
    CodexResponseItem::FunctionCallOutput { call_id, output } => {
      vec![tool_output_event(session_id, call_id, None, output, timestamp)]
    }
    CodexResponseItem::LocalShellCall {
      id,
      call_id,
      status,
      action,
    } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: id,
      parent_id: None,
      tool_call_id: call_id,
      tool_name: Some("local_shell".to_string()),
      phase: Phase::Finished,
      input: Some(action),
      output: status.map(Value::String),
      is_error: None,
      timestamp,
    })],
    CodexResponseItem::CustomToolCall {
      id,
      status,
      call_id,
      name,
      input,
    } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: id,
      parent_id: None,
      tool_call_id: Some(call_id),
      tool_name: Some(name),
      phase: Phase::Finished,
      input: Some(parse_json_string_or_text(input)),
      output: status.map(Value::String),
      is_error: None,
      timestamp,
    })],
    CodexResponseItem::CustomToolCallOutput { call_id, name, output } => {
      vec![tool_output_event(session_id, call_id, name, output, timestamp)]
    }
    CodexResponseItem::WebSearchCall {
      id,
      call_id,
      status,
      action,
    } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: id,
      parent_id: None,
      tool_call_id: call_id,
      tool_name: Some("web_search".to_string()),
      phase: Phase::Finished,
      input: Some(action),
      output: status.map(Value::String),
      is_error: None,
      timestamp,
    })],
    CodexResponseItem::ToolSearchCall {
      id,
      call_id,
      status,
      execution,
      arguments,
    } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: id,
      parent_id: None,
      tool_call_id: call_id,
      tool_name: Some(format!("tool_search.{execution}")),
      phase: Phase::Finished,
      input: Some(arguments),
      output: status.map(Value::String),
      is_error: None,
      timestamp,
    })],
    CodexResponseItem::ToolSearchOutput {
      call_id,
      status,
      execution,
      tools,
    } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: call_id,
      tool_name: Some(format!("tool_search.{execution}")),
      phase: Phase::Finished,
      input: None,
      output: Some(serde_json::json!({ "status": status, "tools": tools })),
      is_error: None,
      timestamp,
    })],
    CodexResponseItem::Unknown(value) => vec![unknown_event(
      session_id,
      unknown_type("response_item", &value),
      timestamp,
    )],
  }
}

fn normalize_message(
  session_id: Option<String>,
  message_id: Option<String>,
  role: String,
  content: Vec<CodexContentItem>,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  if role != "assistant" {
    return Vec::new();
  }

  let text = content
    .into_iter()
    .filter_map(content_item_text)
    .collect::<Vec<_>>()
    .join("\n");
  if text.is_empty() {
    return vec![unknown_event(
      session_id,
      Some("response_item.message".to_string()),
      timestamp,
    )];
  }

  vec![AgentEvent::Message(MessageEvent {
    provider: Provider::Codex,
    session_id,
    message_id,
    parent_id: None,
    role: codex_role(&role),
    phase: Phase::Finished,
    text,
    timestamp,
  })]
}

fn normalize_event_msg(session_id: Option<String>, event: CodexEventMsg, timestamp: Option<String>) -> Vec<AgentEvent> {
  match event {
    CodexEventMsg::TaskStarted { turn_id } | CodexEventMsg::TurnStarted { turn_id } => {
      let _ = turn_id;
      Vec::new()
    }
    CodexEventMsg::TaskComplete { turn_id } => {
      let _ = turn_id;
      Vec::new()
    }
    CodexEventMsg::UserMessage { message } => vec![message_event(session_id, Role::User, message, timestamp)],
    CodexEventMsg::AgentMessage { message, phase } => {
      let _ = (message, phase);
      Vec::new()
    }
    CodexEventMsg::AgentReasoning { text, message } => {
      let text = text.or(message).unwrap_or_default();
      if text.is_empty() {
        vec![unknown_event(
          session_id,
          Some("event_msg.agent_reasoning".to_string()),
          timestamp,
        )]
      } else {
        vec![AgentEvent::Reasoning(ReasoningEvent {
          provider: Provider::Codex,
          session_id,
          message_id: None,
          parent_id: None,
          phase: Phase::Finished,
          text,
          timestamp,
        })]
      }
    }
    CodexEventMsg::ExecCommandBegin { call_id, command } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: call_id,
      tool_name: Some("exec_command".to_string()),
      phase: Phase::Started,
      input: Some(Value::Array(command.into_iter().map(Value::String).collect())),
      output: None,
      is_error: None,
      timestamp,
    })],
    CodexEventMsg::ExecCommandEnd { call_id, status } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: call_id,
      tool_name: Some("exec_command".to_string()),
      phase: Phase::Finished,
      input: None,
      output: status.map(Value::String),
      is_error: None,
      timestamp,
    })],
    CodexEventMsg::McpToolCallBegin {
      call_id,
      name,
      arguments,
    } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: call_id,
      tool_name: name,
      phase: Phase::Started,
      input: arguments,
      output: None,
      is_error: None,
      timestamp,
    })],
    CodexEventMsg::McpToolCallEnd {
      call_id,
      name,
      result,
      error,
    } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: call_id,
      tool_name: name,
      phase: Phase::Finished,
      input: None,
      output: result.or_else(|| error.clone()),
      is_error: Some(error.is_some()),
      timestamp,
    })],
    CodexEventMsg::Error { message } => vec![AgentEvent::Error(ErrorEvent {
      provider: Provider::Codex,
      session_id,
      message,
      timestamp,
    })],
    CodexEventMsg::PatchApplyBegin { call_id, changes } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: Some(call_id),
      tool_name: Some("apply_patch".to_string()),
      phase: Phase::Started,
      input: Some(changes),
      output: None,
      is_error: None,
      timestamp,
    })],
    CodexEventMsg::PatchApplyEnd {
      call_id,
      stdout,
      stderr,
      success,
      status,
    } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: Some(call_id),
      tool_name: Some("apply_patch".to_string()),
      phase: Phase::Finished,
      input: None,
      output: Some(serde_json::json!({
        "status": status,
        "stdout": stdout,
        "stderr": stderr,
      })),
      is_error: Some(!success),
      timestamp,
    })],
    CodexEventMsg::TokenCount {} => Vec::new(),
    CodexEventMsg::TurnComplete {} => Vec::new(),
    CodexEventMsg::TurnAborted { reason } => vec![AgentEvent::Error(ErrorEvent {
      provider: Provider::Codex,
      session_id,
      message: reason.unwrap_or_else(|| "turn aborted".to_string()),
      timestamp,
    })],
    CodexEventMsg::Unknown(value) => vec![unknown_event(session_id, unknown_type("event_msg", &value), timestamp)],
  }
}

fn normalize_compacted(
  session_id: Option<String>,
  event: CodexCompacted,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  vec![AgentEvent::Message(MessageEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    role: Role::Assistant,
    phase: Phase::Finished,
    text: event.message,
    timestamp,
  })]
}

fn content_item_text(item: CodexContentItem) -> Option<String> {
  match item {
    CodexContentItem::InputText { text } | CodexContentItem::OutputText { text } | CodexContentItem::Text { text } => {
      Some(text)
    }
    CodexContentItem::Unknown(_) => None,
  }
}

fn reasoning_content_text(item: CodexReasoningContent) -> Option<String> {
  match item {
    CodexReasoningContent::Text { text } => Some(text),
    CodexReasoningContent::Unknown(_) => None,
  }
}

fn codex_role(role: &str) -> Role {
  match role {
    "user" => Role::User,
    "assistant" => Role::Assistant,
    "system" | "developer" => Role::System,
    "tool" => Role::Tool,
    _ => Role::Unknown,
  }
}

fn message_event(session_id: Option<String>, role: Role, text: String, timestamp: Option<String>) -> AgentEvent {
  AgentEvent::Message(MessageEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    role,
    phase: Phase::Finished,
    text,
    timestamp,
  })
}

fn tool_output_event(
  session_id: Option<String>,
  call_id: String,
  name: Option<String>,
  output: Value,
  timestamp: Option<String>,
) -> AgentEvent {
  AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    tool_call_id: Some(call_id),
    tool_name: name,
    phase: Phase::Finished,
    input: None,
    output: Some(output),
    is_error: None,
    timestamp,
  })
}

fn parse_json_string_or_text(value: String) -> Value {
  serde_json::from_str(&value).unwrap_or(Value::String(value))
}

fn unknown_event(session_id: Option<String>, native_type: Option<String>, timestamp: Option<String>) -> AgentEvent {
  AgentEvent::Unknown(UnknownEvent {
    provider: Provider::Codex,
    session_id,
    native_type,
    timestamp,
  })
}

fn unknown_type(prefix: &str, value: &Value) -> Option<String> {
  let suffix = value.get("type").and_then(Value::as_str).unwrap_or("unknown");
  Some(format!("{prefix}.{suffix}"))
}
