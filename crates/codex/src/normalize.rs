use serde_json::Value;

use crate::event::{
  CodexCompacted, CodexContentItem, CodexEvent, CodexEventMsg, CodexLine, CodexReasoningContent, CodexResponseItem,
  CodexSessionMeta,
};
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
        .filter_map(|summary| summary.text)
        .collect::<Vec<_>>()
        .join("\n");
      if text.is_empty() && summary.is_empty() && encrypted_content.is_none() {
        Vec::new()
      } else {
        vec![AgentEvent::Reasoning(ReasoningEvent {
          provider: Provider::Codex,
          session_id,
          message_id: id,
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
      namespace,
      arguments,
      call_id,
    } => {
      let tool_name = namespace.map_or(name.clone(), |namespace| format!("{namespace}.{name}"));
      let input = parse_json_string_or_text(arguments);
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: id,
        parent_id: None,
        tool_call_id: Some(call_id),
        tool_name: Some(tool_name.clone()),
        tool_kind: tool_kind_for_name(&tool_name),
        summary: tool_summary_for_input(&tool_name, &input),
        phase: Phase::Finished,
        input: Some(input),
        output: None,
        is_error: None,
        timestamp,
      })]
    }
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
      tool_kind: ToolKind::Shell,
      summary: tool_summary_for_input("local_shell", &action),
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
      tool_kind: tool_kind_for_name("web_search"),
      summary: tool_summary_for_input("web_search", &action),
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
      tool_kind: ToolKind::Search,
      summary: tool_summary_for_input("search", &arguments),
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
      tool_kind: ToolKind::Search,
      summary: None,
      phase: Phase::Finished,
      input: None,
      output: Some(serde_json::json!({ "status": status, "tools": tools })),
      is_error: None,
      timestamp,
    })],
    CodexResponseItem::Unknown(value) => vec![unknown_event(
      session_id,
      unknown_type("response_item", &value),
      Some(value),
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
          text: present_text(text),
          summary: None,
          encrypted_content: None,
          signature: None,
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
      tool_kind: ToolKind::Shell,
      summary: Some(ToolSummary::Shell {
        command: Some(command.join(" ")),
        cwd: None,
        exit_code: None,
      }),
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
      tool_kind: ToolKind::Shell,
      summary: Some(ToolSummary::Shell {
        command: None,
        cwd: None,
        exit_code: None,
      }),
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
      tool_name: name.clone(),
      tool_kind: tool_kind_for_optional_name(name.as_deref()),
      summary: tool_summary_for_io(name.as_deref(), arguments.as_ref(), None),
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
      tool_name: name.clone(),
      tool_kind: tool_kind_for_optional_name(name.as_deref()),
      summary: None,
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
      tool_kind: ToolKind::FileEdit,
      summary: Some(patch_summary(&changes)),
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
      changes,
      status,
    } => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: None,
      parent_id: None,
      tool_call_id: Some(call_id),
      tool_name: Some("apply_patch".to_string()),
      tool_kind: ToolKind::FileEdit,
      summary: changes.as_ref().map(patch_summary),
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
    CodexEventMsg::ThreadGoalUpdated {
      thread_id,
      turn_id,
      goal,
    } => vec![AgentEvent::GoalUpdated(GoalUpdated {
      provider: Provider::Codex,
      session_id: thread_id.or(session_id),
      turn_id,
      goal,
      timestamp,
    })],
    CodexEventMsg::TurnComplete {} => Vec::new(),
    CodexEventMsg::TurnAborted { reason } => vec![AgentEvent::Error(ErrorEvent {
      provider: Provider::Codex,
      session_id,
      message: reason.unwrap_or_else(|| "turn aborted".to_string()),
      timestamp,
    })],
    CodexEventMsg::Unknown(value) => {
      vec![unknown_event(
        session_id,
        unknown_type("event_msg", &value),
        Some(value),
        timestamp,
      )]
    }
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
    CodexReasoningContent::ReasoningText { text } | CodexReasoningContent::Text { text } => Some(text),
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

fn unknown_type(prefix: &str, value: &Value) -> Option<String> {
  let suffix = value.get("type").and_then(Value::as_str).unwrap_or("unknown");
  Some(format!("{prefix}.{suffix}"))
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
