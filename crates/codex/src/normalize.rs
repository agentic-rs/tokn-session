use codex_protocol::models::{
  ContentItem as CodexContentItem, ReasoningItemContent as CodexReasoningContent, ReasoningItemReasoningSummary,
  ResponseItem as CodexResponseItem,
};
use codex_protocol::protocol::{
  CompactedItem as CodexCompacted, EventMsg as CodexEventMsg, RolloutItem, SessionMetaLine as CodexSessionMeta,
};
use serde_json::Value;

use crate::event::{CodexEvent, CodexLine};
use tokn_session_core::{
  AgentEvent, ErrorEvent, GoalUpdated, MessageEvent, Phase, Provider, ProviderChanged, ReasoningEvent, Role,
  SessionStarted, ToolCallEvent, ToolKind, ToolSummary, UnknownEvent, patch_summary, tool_kind_for_name,
  tool_kind_for_optional_name, tool_summary_for_input, tool_summary_for_io,
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
      CodexEvent::RolloutItem(event) => match event {
        RolloutItem::SessionMeta(event) => self.normalize_session_meta(event, timestamp),
        RolloutItem::ResponseItem(event) => normalize_response_item(self.session_id.clone(), event, timestamp),
        RolloutItem::EventMsg(event) => normalize_event_msg(self.session_id.clone(), event, timestamp),
        RolloutItem::TurnContext(_) => Vec::new(),
        RolloutItem::Compacted(event) => normalize_compacted(self.session_id.clone(), event, timestamp),
      },
      CodexEvent::Unknown(value) => self.normalize_unknown(value, timestamp),
    }
  }

  fn normalize_session_meta(&mut self, event: CodexSessionMeta, line_timestamp: Option<String>) -> Vec<AgentEvent> {
    let meta = event.meta;
    let session_id = meta.id.to_string();
    self.session_id = Some(session_id.clone());
    let timestamp = Some(meta.timestamp).or(line_timestamp);
    let mut events = vec![AgentEvent::SessionStarted(SessionStarted {
      provider: Provider::Codex,
      session_id,
      cwd: Some(meta.cwd.display().to_string()),
      timestamp: timestamp.clone(),
    })];

    if meta.model_provider.is_some() {
      events.push(AgentEvent::ProviderChanged(ProviderChanged {
        provider: Provider::Codex,
        session_id: self.session_id.clone(),
        native_id: Some(meta.id.to_string()),
        native_parent_id: None,
        model_provider: meta.model_provider,
        model_id: None,
        thinking_level: None,
        timestamp,
      }));
    }

    events
  }

  fn normalize_unknown(&mut self, value: Value, timestamp: Option<String>) -> Vec<AgentEvent> {
    if value.get("type").and_then(Value::as_str) == Some("session_meta") {
      if let Some(payload) = value.get("payload") {
        if let Some(id) = payload.get("id").and_then(Value::as_str) {
          self.session_id = Some(id.to_string());
          let timestamp = payload
            .get("timestamp")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(timestamp);
          let mut events = vec![AgentEvent::SessionStarted(SessionStarted {
            provider: Provider::Codex,
            session_id: id.to_string(),
            cwd: payload.get("cwd").and_then(Value::as_str).map(str::to_string),
            timestamp: timestamp.clone(),
          })];

          if let Some(model_provider) = payload.get("model_provider").and_then(Value::as_str) {
            events.push(AgentEvent::ProviderChanged(ProviderChanged {
              provider: Provider::Codex,
              session_id: self.session_id.clone(),
              native_id: Some(id.to_string()),
              native_parent_id: payload
                .get("parent_thread_id")
                .and_then(Value::as_str)
                .map(str::to_string),
              model_provider: Some(model_provider.to_string()),
              model_id: None,
              thinking_level: None,
              timestamp,
            }));
          }

          return events;
        }
      }
    }

    normalize_unknown_line(self.session_id.clone(), value, timestamp)
  }
}

fn normalize_response_item(
  session_id: Option<String>,
  event: CodexResponseItem,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  match event {
    CodexResponseItem::Message { id, role, content } => normalize_message(session_id, id, role, content, timestamp),
    CodexResponseItem::Reasoning {
      id,
      summary,
      content,
      encrypted_content,
    } => {
      let text = content
        .unwrap_or_default()
        .into_iter()
        .filter_map(reasoning_content_text)
        .collect::<Vec<_>>()
        .join("\n");
      let summary = summary
        .into_iter()
        .map(reasoning_summary_text)
        .collect::<Vec<_>>()
        .join("\n");
      if text.is_empty() && summary.is_empty() && encrypted_content.is_none() {
        Vec::new()
      } else {
        vec![AgentEvent::Reasoning(ReasoningEvent {
          provider: Provider::Codex,
          session_id,
          message_id: Some(id),
          parent_id: None,
          phase: Phase::Finished,
          text: present_text(text),
          summary: present_text(summary),
          encrypted_content,
          signature: None,
          timestamp,
        })]
      }
    }
    CodexResponseItem::FunctionCall {
      id,
      name,
      arguments,
      call_id,
    } => {
      let input = parse_json_string_or_text(arguments);
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: id,
        parent_id: None,
        tool_call_id: Some(call_id),
        tool_name: Some(name.clone()),
        tool_kind: tool_kind_for_name(&name),
        summary: tool_summary_for_input(&name, &input),
        phase: Phase::Finished,
        input: Some(input),
        output: None,
        is_error: None,
        timestamp,
      })]
    }
    CodexResponseItem::FunctionCallOutput { call_id, output } => vec![tool_output_event(
      session_id,
      call_id,
      None,
      serde_json::to_value(output).unwrap_or(Value::Null),
      timestamp,
    )],
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
      tool_kind: ToolKind::Shell,
      summary: tool_summary_for_input("local_shell", &serde_json::to_value(&action).unwrap_or(Value::Null)),
      phase: Phase::Finished,
      input: Some(serde_json::to_value(action).unwrap_or(Value::Null)),
      output: Some(serde_json::to_value(status).unwrap_or(Value::Null)),
      is_error: None,
      timestamp,
    })],
    CodexResponseItem::CustomToolCall {
      id,
      status,
      call_id,
      name,
      input,
    } => {
      let input = parse_json_string_or_text(input);
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: id,
        parent_id: None,
        tool_call_id: Some(call_id),
        tool_name: Some(name.clone()),
        tool_kind: tool_kind_for_name(&name),
        summary: tool_summary_for_input(&name, &input),
        phase: Phase::Finished,
        input: Some(input),
        output: status.map(Value::String),
        is_error: None,
        timestamp,
      })]
    }
    CodexResponseItem::CustomToolCallOutput { call_id, output } => vec![tool_output_event(
      session_id,
      call_id,
      None,
      Value::String(output),
      timestamp,
    )],
    CodexResponseItem::WebSearchCall { id, status, action } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: id,
      parent_id: None,
      tool_call_id: None,
      tool_name: Some("web_search".to_string()),
      tool_kind: tool_kind_for_name("web_search"),
      summary: tool_summary_for_input("web_search", &serde_json::to_value(&action).unwrap_or(Value::Null)),
      phase: Phase::Finished,
      input: Some(serde_json::to_value(action).unwrap_or(Value::Null)),
      output: status.map(Value::String),
      is_error: None,
      timestamp,
    })],
    event @ (CodexResponseItem::GhostSnapshot { .. }
    | CodexResponseItem::CompactionSummary { .. }
    | CodexResponseItem::Other) => vec![unknown_event(
      session_id,
      Some("response_item.unknown".to_string()),
      Some(serde_json::to_value(event).unwrap_or(Value::Null)),
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
      None,
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
    CodexEventMsg::TaskStarted(_) | CodexEventMsg::TaskComplete(_) => Vec::new(),
    CodexEventMsg::UserMessage(event) => vec![message_event(session_id, Role::User, event.message, timestamp)],
    CodexEventMsg::AgentMessage(event) => {
      let _ = event.message;
      Vec::new()
    }
    CodexEventMsg::AgentMessageDelta(event) => vec![AgentEvent::Message(MessageEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      role: Role::Assistant,
      phase: Phase::Delta,
      text: event.delta,
      timestamp,
    })],
    CodexEventMsg::AgentReasoning(event) => {
      if event.text.is_empty() {
        vec![unknown_event(
          session_id,
          Some("event_msg.agent_reasoning".to_string()),
          None,
          timestamp,
        )]
      } else {
        vec![AgentEvent::Reasoning(ReasoningEvent {
          provider: Provider::Codex,
          session_id,
          message_id: None,
          parent_id: None,
          phase: Phase::Finished,
          text: present_text(event.text),
          summary: None,
          encrypted_content: None,
          signature: None,
          timestamp,
        })]
      }
    }
    CodexEventMsg::AgentReasoningDelta(event) => vec![AgentEvent::Reasoning(ReasoningEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      phase: Phase::Delta,
      text: present_text(event.delta),
      summary: None,
      encrypted_content: None,
      signature: None,
      timestamp,
    })],
    CodexEventMsg::AgentReasoningRawContent(event) => vec![AgentEvent::Reasoning(ReasoningEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      phase: Phase::Finished,
      text: present_text(event.text),
      summary: None,
      encrypted_content: None,
      signature: None,
      timestamp,
    })],
    CodexEventMsg::AgentReasoningRawContentDelta(event) => vec![AgentEvent::Reasoning(ReasoningEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      phase: Phase::Delta,
      text: present_text(event.delta),
      summary: None,
      encrypted_content: None,
      signature: None,
      timestamp,
    })],
    CodexEventMsg::ExecCommandBegin(event) => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: Some(event.call_id),
      tool_name: Some("exec_command".to_string()),
      tool_kind: ToolKind::Shell,
      summary: Some(ToolSummary::Shell {
        command: Some(event.command.join(" ")),
        cwd: Some(event.cwd.display().to_string()),
        exit_code: None,
      }),
      phase: Phase::Started,
      input: Some(Value::Array(event.command.into_iter().map(Value::String).collect())),
      output: None,
      is_error: None,
      timestamp,
    })],
    CodexEventMsg::ExecCommandOutputDelta(event) => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: Some(event.call_id.clone()),
      tool_name: Some("exec_command".to_string()),
      tool_kind: ToolKind::Shell,
      summary: None,
      phase: Phase::Delta,
      input: None,
      output: Some(serde_json::to_value(event).unwrap_or(Value::Null)),
      is_error: None,
      timestamp,
    })],
    CodexEventMsg::ExecCommandEnd(event) => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: Some(event.call_id),
      tool_name: Some("exec_command".to_string()),
      tool_kind: ToolKind::Shell,
      summary: Some(ToolSummary::Shell {
        command: Some(event.command.join(" ")),
        cwd: Some(event.cwd.display().to_string()),
        exit_code: Some(event.exit_code.into()),
      }),
      phase: Phase::Finished,
      input: None,
      output: Some(serde_json::json!({
        "stdout": event.stdout,
        "stderr": event.stderr,
        "aggregated_output": event.aggregated_output,
        "formatted_output": event.formatted_output,
      })),
      is_error: Some(event.exit_code != 0),
      timestamp,
    })],
    CodexEventMsg::McpToolCallBegin(event) => {
      let name = format!("{}.{}", event.invocation.server, event.invocation.tool);
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: None,
        parent_id: None,
        tool_call_id: Some(event.call_id),
        tool_name: Some(name.clone()),
        tool_kind: tool_kind_for_name(&name),
        summary: tool_summary_for_io(Some(&name), event.invocation.arguments.as_ref(), None),
        phase: Phase::Started,
        input: event.invocation.arguments,
        output: None,
        is_error: None,
        timestamp,
      })]
    }
    CodexEventMsg::McpToolCallEnd(event) => {
      let name = format!("{}.{}", event.invocation.server, event.invocation.tool);
      let is_success = event.is_success();
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: None,
        parent_id: None,
        tool_call_id: Some(event.call_id),
        tool_name: Some(name.clone()),
        tool_kind: tool_kind_for_name(&name),
        summary: None,
        phase: Phase::Finished,
        input: None,
        output: Some(serde_json::to_value(event.result).unwrap_or(Value::Null)),
        is_error: Some(!is_success),
        timestamp,
      })]
    }
    CodexEventMsg::WebSearchBegin(event) => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: Some(event.call_id),
      tool_name: Some("web_search".to_string()),
      tool_kind: ToolKind::Search,
      summary: None,
      phase: Phase::Started,
      input: None,
      output: None,
      is_error: None,
      timestamp,
    })],
    CodexEventMsg::WebSearchEnd(event) => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: Some(event.call_id),
      tool_name: Some("web_search".to_string()),
      tool_kind: ToolKind::Search,
      summary: Some(ToolSummary::Search {
        query: Some(event.query.clone()),
      }),
      phase: Phase::Finished,
      input: None,
      output: Some(serde_json::json!({ "query": event.query })),
      is_error: None,
      timestamp,
    })],
    CodexEventMsg::Error(event) => vec![AgentEvent::Error(ErrorEvent {
      provider: Provider::Codex,
      session_id,
      message: event.message,
      timestamp,
    })],
    CodexEventMsg::Warning(event) => vec![AgentEvent::Error(ErrorEvent {
      provider: Provider::Codex,
      session_id,
      message: event.message,
      timestamp,
    })],
    CodexEventMsg::PatchApplyBegin(event) => {
      let changes = serde_json::to_value(event.changes).unwrap_or(Value::Null);
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: None,
        parent_id: None,
        tool_call_id: Some(event.call_id),
        tool_name: Some("apply_patch".to_string()),
        tool_kind: ToolKind::FileEdit,
        summary: Some(patch_summary(&changes)),
        phase: Phase::Started,
        input: Some(changes),
        output: None,
        is_error: None,
        timestamp,
      })]
    }
    CodexEventMsg::PatchApplyEnd(event) => {
      let changes = serde_json::to_value(event.changes).unwrap_or(Value::Null);
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: None,
        parent_id: None,
        tool_call_id: Some(event.call_id),
        tool_name: Some("apply_patch".to_string()),
        tool_kind: ToolKind::FileEdit,
        summary: Some(patch_summary(&changes)),
        phase: Phase::Finished,
        input: None,
        output: Some(serde_json::json!({
          "stdout": event.stdout,
          "stderr": event.stderr,
        })),
        is_error: Some(!event.success),
        timestamp,
      })]
    }
    CodexEventMsg::ViewImageToolCall(event) => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: Some(event.call_id),
      tool_name: Some("view_image".to_string()),
      tool_kind: ToolKind::FileRead,
      summary: Some(ToolSummary::FileRead {
        path: Some(event.path.display().to_string()),
      }),
      phase: Phase::Finished,
      input: Some(serde_json::json!({ "path": event.path })),
      output: None,
      is_error: None,
      timestamp,
    })],
    CodexEventMsg::TokenCount(_) => Vec::new(),
    CodexEventMsg::TurnAborted(event) => vec![AgentEvent::Error(ErrorEvent {
      provider: Provider::Codex,
      session_id,
      message: format!("{:?}", event.reason),
      timestamp,
    })],
    event => vec![unknown_event(
      session_id,
      Some("event_msg.unknown".to_string()),
      Some(serde_json::to_value(event).unwrap_or(Value::Null)),
      timestamp,
    )],
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
    CodexContentItem::InputText { text } | CodexContentItem::OutputText { text } => Some(text),
    CodexContentItem::InputImage { .. } => None,
  }
}

fn reasoning_summary_text(item: ReasoningItemReasoningSummary) -> String {
  match item {
    ReasoningItemReasoningSummary::SummaryText { text } => text,
  }
}

fn reasoning_content_text(item: CodexReasoningContent) -> Option<String> {
  match item {
    CodexReasoningContent::ReasoningText { text } | CodexReasoningContent::Text { text } => Some(text),
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
    tool_name: name.clone(),
    tool_kind: tool_kind_for_optional_name(name.as_deref()),
    summary: None,
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

fn present_text(text: String) -> Option<String> {
  (!text.is_empty()).then_some(text)
}

fn unknown_event(
  session_id: Option<String>,
  native_type: Option<String>,
  native: Option<Value>,
  timestamp: Option<String>,
) -> AgentEvent {
  AgentEvent::Unknown(UnknownEvent {
    provider: Provider::Codex,
    session_id,
    native_type,
    native,
    timestamp,
  })
}

fn normalize_unknown_line(session_id: Option<String>, value: Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  if value.get("type").and_then(Value::as_str) == Some("response_item") {
    if let Some(payload) = value.get("payload") {
      match payload.get("type").and_then(Value::as_str) {
        Some("reasoning") => {
          let text = payload
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
          let summary = payload
            .get("summary")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
          let encrypted_content = payload
            .get("encrypted_content")
            .and_then(Value::as_str)
            .map(str::to_string);
          if !text.is_empty() || !summary.is_empty() || encrypted_content.is_some() {
            return vec![AgentEvent::Reasoning(ReasoningEvent {
              provider: Provider::Codex,
              session_id,
              message_id: payload.get("id").and_then(Value::as_str).map(str::to_string),
              parent_id: None,
              phase: Phase::Finished,
              text: present_text(text),
              summary: present_text(summary),
              encrypted_content,
              signature: None,
              timestamp,
            })];
          }
        }
        Some("message") => {
          if payload.get("role").and_then(Value::as_str) == Some("assistant") {
            let text = payload
              .get("content")
              .and_then(Value::as_array)
              .into_iter()
              .flatten()
              .filter_map(|item| item.get("text").and_then(Value::as_str))
              .collect::<Vec<_>>()
              .join("\n");
            if !text.is_empty() {
              return vec![AgentEvent::Message(MessageEvent {
                provider: Provider::Codex,
                session_id,
                message_id: payload.get("id").and_then(Value::as_str).map(str::to_string),
                parent_id: None,
                role: Role::Assistant,
                phase: Phase::Finished,
                text,
                timestamp,
              })];
            }
          }
        }
        _ => {}
      }
    }
  }

  if value.get("type").and_then(Value::as_str) == Some("event_msg") {
    if let Some(payload) = value.get("payload") {
      match payload.get("type").and_then(Value::as_str) {
        Some("thread_goal_updated") => {
          return vec![AgentEvent::GoalUpdated(GoalUpdated {
            provider: Provider::Codex,
            session_id: payload
              .get("threadId")
              .or_else(|| payload.get("thread_id"))
              .and_then(Value::as_str)
              .map(str::to_string)
              .or(session_id),
            turn_id: payload
              .get("turnId")
              .or_else(|| payload.get("turn_id"))
              .and_then(Value::as_str)
              .map(str::to_string),
            goal: payload.get("goal").cloned(),
            timestamp,
          })];
        }
        Some("exec_command_begin") => {
          let call_id = payload.get("call_id").and_then(Value::as_str).map(str::to_string);
          let command = payload
            .get("command")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>();
          if !command.is_empty() {
            return vec![AgentEvent::ToolCall(ToolCallEvent {
              provider: Provider::Codex,
              session_id,
              message_id: None,
              parent_id: None,
              tool_call_id: call_id,
              tool_name: Some("exec_command".to_string()),
              tool_kind: ToolKind::Shell,
              summary: Some(ToolSummary::Shell {
                command: Some(command.join(" ")),
                cwd: payload.get("cwd").and_then(Value::as_str).map(str::to_string),
                exit_code: None,
              }),
              phase: Phase::Started,
              input: Some(Value::Array(command.into_iter().map(Value::String).collect())),
              output: None,
              is_error: None,
              timestamp,
            })];
          }
        }
        Some("patch_apply_begin") => {
          let changes = payload.get("changes").cloned().unwrap_or(Value::Null);
          return vec![AgentEvent::ToolCall(ToolCallEvent {
            provider: Provider::Codex,
            session_id,
            message_id: None,
            parent_id: None,
            tool_call_id: payload.get("call_id").and_then(Value::as_str).map(str::to_string),
            tool_name: Some("apply_patch".to_string()),
            tool_kind: ToolKind::FileEdit,
            summary: Some(patch_summary(&changes)),
            phase: Phase::Started,
            input: Some(changes),
            output: None,
            is_error: None,
            timestamp,
          })];
        }
        _ => {}
      }
    }
  }

  vec![unknown_event(
    session_id,
    unknown_type_for_line(&value),
    Some(value.get("payload").cloned().unwrap_or(value)),
    timestamp,
  )]
}

fn unknown_type_for_line(value: &Value) -> Option<String> {
  let line_type = value.get("type").and_then(Value::as_str)?;
  let payload_type = value
    .get("payload")
    .and_then(|payload| payload.get("type"))
    .and_then(Value::as_str);

  Some(match payload_type {
    Some(payload_type) => format!("{line_type}.{payload_type}"),
    None => line_type.to_string(),
  })
}

#[cfg(test)]
mod tests {
  use serde_json::Value;
  use tokn_session_core::{AgentEvent, Phase, Role, ToolKind, ToolSummary};

  use super::*;
  use crate::event::CodexLine;

  #[test]
  fn normalizes_basic_fixture_events() {
    let events = normalize_fixture(include_str!("../fixtures/basic_session.jsonl"));

    assert_eq!(events.len(), 9);
    assert_session_started(&events[0]);
    assert_provider_changed(&events[1]);
    assert_user_message(&events[2]);
    assert_assistant_message(&events[3]);
    assert_reasoning(&events[4]);
    assert_shell_tool_started(&events[5]);
    assert_patch_tool_started(&events[6]);
    assert_goal_updated(&events[7]);
    assert_unknown_event(&events[8]);
  }

  fn normalize_fixture(input: &str) -> Vec<AgentEvent> {
    let mut normalizer = CodexNormalizer::new();
    input
      .lines()
      .filter(|line| !line.trim().is_empty())
      .flat_map(|line| {
        let line: CodexLine = serde_json::from_str(line).expect("fixture line should parse");
        normalizer.normalize(line)
      })
      .collect()
  }

  fn assert_session_started(event: &AgentEvent) {
    let AgentEvent::SessionStarted(event) = event else {
      panic!("expected session started event");
    };
    assert_eq!(event.session_id, "session-fixture");
    assert_eq!(event.cwd.as_deref(), Some("/tmp/project"));
    assert_eq!(event.timestamp.as_deref(), Some("2026-06-04T00:00:00Z"));
  }

  fn assert_provider_changed(event: &AgentEvent) {
    let AgentEvent::ProviderChanged(event) = event else {
      panic!("expected provider changed event");
    };
    assert_eq!(event.session_id.as_deref(), Some("session-fixture"));
    assert_eq!(event.model_provider.as_deref(), Some("openai"));
  }

  fn assert_user_message(event: &AgentEvent) {
    let AgentEvent::Message(event) = event else {
      panic!("expected user message event");
    };
    assert!(matches!(event.role, Role::User));
    assert_eq!(event.text, "build a tiny test");
    assert_eq!(event.session_id.as_deref(), Some("session-fixture"));
  }

  fn assert_assistant_message(event: &AgentEvent) {
    let AgentEvent::Message(event) = event else {
      panic!("expected assistant message event");
    };
    assert!(matches!(event.role, Role::Assistant));
    assert_eq!(event.message_id.as_deref(), Some("msg-assistant"));
    assert_eq!(event.text, "done");
  }

  fn assert_reasoning(event: &AgentEvent) {
    let AgentEvent::Reasoning(event) = event else {
      panic!("expected reasoning event");
    };
    assert_eq!(event.message_id.as_deref(), Some("rsn-1"));
    assert_eq!(event.summary.as_deref(), Some("checked files"));
    assert_eq!(event.text.as_deref(), Some("thinking out loud"));
    assert_eq!(event.encrypted_content.as_deref(), Some("ciphertext"));
  }

  fn assert_shell_tool_started(event: &AgentEvent) {
    let AgentEvent::ToolCall(event) = event else {
      panic!("expected shell tool event");
    };
    assert_eq!(event.tool_call_id.as_deref(), Some("call-shell"));
    assert_eq!(event.tool_name.as_deref(), Some("exec_command"));
    assert!(matches!(event.tool_kind, ToolKind::Shell));
    assert!(matches!(event.phase, Phase::Started));
    match event.summary.as_ref() {
      Some(ToolSummary::Shell {
        command,
        cwd,
        exit_code,
      }) => {
        assert_eq!(command.as_deref(), Some("cargo test"));
        assert_eq!(cwd, &None);
        assert_eq!(exit_code, &None);
      }
      _ => panic!("expected shell summary"),
    }
  }

  fn assert_patch_tool_started(event: &AgentEvent) {
    let AgentEvent::ToolCall(event) = event else {
      panic!("expected patch tool event");
    };
    assert_eq!(event.tool_call_id.as_deref(), Some("call-edit"));
    assert_eq!(event.tool_name.as_deref(), Some("apply_patch"));
    assert!(matches!(event.tool_kind, ToolKind::FileEdit));
    match event.summary.as_ref() {
      Some(ToolSummary::FileEdit { path, added, removed }) => {
        assert_eq!(path.as_deref(), Some("crates/core/src/lib.rs"));
        assert_eq!(added, &Some(2));
        assert_eq!(removed, &Some(1));
      }
      _ => panic!("expected file edit summary"),
    }
  }

  fn assert_goal_updated(event: &AgentEvent) {
    let AgentEvent::GoalUpdated(event) = event else {
      panic!("expected goal updated event");
    };
    assert_eq!(event.session_id.as_deref(), Some("session-fixture"));
    assert_eq!(event.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(
      event.goal.as_ref().and_then(|goal| goal.get("status")),
      Some(&Value::String("complete".to_string()))
    );
  }

  fn assert_unknown_event(event: &AgentEvent) {
    let AgentEvent::Unknown(event) = event else {
      panic!("expected unknown event");
    };
    assert_eq!(event.session_id.as_deref(), Some("session-fixture"));
    assert_eq!(event.native_type.as_deref(), Some("event_msg.new_native_event"));
    assert_eq!(
      event.native.as_ref().and_then(|native| native.get("value")),
      Some(&Value::Number(123.into()))
    );
  }
}
