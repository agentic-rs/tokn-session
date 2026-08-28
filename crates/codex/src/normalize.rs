use serde_json::{Value, json};
use tokn_codex_protocol::{
  AgentMessageItem, CompactedItem, ContentItem, EventMessage, InterAgentCommunicationItem, MessageItem, ReasoningItem,
  ResponseItem, RolloutItem, SessionMetaItem, UnknownItem,
};
use tokn_session_core::{
  AgentActivity, AgentEvent, ErrorEvent, GoalUpdated, MessageDelivery, MessageEvent, Phase, Provider, ProviderChanged,
  ReasoningEvent, Role, SessionHistoryStatus, SessionSettingsApplied, SessionStarted, ToolCallEvent, ToolKind,
  ToolSummary, UnknownEvent, patch_summary, tool_kind_for_name, tool_kind_for_optional_name, tool_summary_for_input,
  tool_summary_for_io,
};

use crate::event::CodexLine;

pub struct CodexNormalizer {
  session_id: Option<String>,
  history_boundary: Option<CodexHistoryBoundary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodexHistoryState {
  BeforeOwner,
  Root,
  AwaitingSubagentBody,
  SubagentBody,
}

pub(crate) struct CodexHistoryBoundary {
  state: CodexHistoryState,
}

impl CodexHistoryBoundary {
  pub(crate) fn new() -> Self {
    Self {
      state: CodexHistoryState::BeforeOwner,
    }
  }

  pub(crate) fn accepts(&mut self, item: &RolloutItem) -> bool {
    match self.state {
      CodexHistoryState::BeforeOwner => {
        if let RolloutItem::SessionMeta(item) = item {
          self.state = if requires_thread_spawn_boundary(item) {
            CodexHistoryState::AwaitingSubagentBody
          } else {
            CodexHistoryState::Root
          };
        }
        true
      }
      CodexHistoryState::Root | CodexHistoryState::SubagentBody => !matches!(item, RolloutItem::SessionMeta(_)),
      CodexHistoryState::AwaitingSubagentBody => match item {
        RolloutItem::InterAgentCommunicationMetadata(item) if item.trigger_turn == Some(true) => {
          self.state = CodexHistoryState::SubagentBody;
          false
        }
        RolloutItem::InterAgentCommunication(item) if item.trigger_turn == Some(true) => {
          self.state = CodexHistoryState::SubagentBody;
          true
        }
        _ => false,
      },
    }
  }

  pub(crate) fn status(&self) -> SessionHistoryStatus {
    match self.state {
      CodexHistoryState::AwaitingSubagentBody => SessionHistoryStatus::SubagentBodyUnavailable,
      CodexHistoryState::SubagentBody => SessionHistoryStatus::FilteredSubagent,
      CodexHistoryState::BeforeOwner | CodexHistoryState::Root => SessionHistoryStatus::Complete,
    }
  }
}

impl CodexNormalizer {
  pub fn new() -> Self {
    Self {
      session_id: None,
      history_boundary: None,
    }
  }

  pub fn new_historical() -> Self {
    Self {
      session_id: None,
      history_boundary: Some(CodexHistoryBoundary::new()),
    }
  }

  pub fn normalize(&mut self, line: CodexLine) -> Vec<AgentEvent> {
    let timestamp = line.timestamp().map(str::to_string);
    let item = line.into_item();

    if self
      .history_boundary
      .as_mut()
      .is_some_and(|boundary| !boundary.accepts(&item))
    {
      return Vec::new();
    }

    self.normalize_item(item, timestamp)
  }

  pub fn history_status(&self) -> SessionHistoryStatus {
    self
      .history_boundary
      .as_ref()
      .map(CodexHistoryBoundary::status)
      .unwrap_or(SessionHistoryStatus::Complete)
  }

  fn normalize_item(&mut self, item: RolloutItem, timestamp: Option<String>) -> Vec<AgentEvent> {
    match item {
      RolloutItem::SessionMeta(item) => self.normalize_session_meta(item, timestamp),
      RolloutItem::ResponseItem(item) => normalize_response_item(self.session_id.clone(), item, timestamp),
      RolloutItem::InterAgentCommunication(item) => {
        normalize_inter_agent_communication(self.session_id.clone(), item, timestamp)
      }
      RolloutItem::InterAgentCommunicationMetadata(_) | RolloutItem::TurnContext(_) | RolloutItem::WorldState(_) => {
        Vec::new()
      }
      RolloutItem::Compacted(item) => normalize_compacted(self.session_id.clone(), item, timestamp),
      RolloutItem::EventMessage(item) => normalize_event_message(self.session_id.clone(), item, timestamp),
      RolloutItem::Unknown(item) => vec![unknown_rollout_event(self.session_id.clone(), item, timestamp)],
    }
  }

  fn normalize_session_meta(&mut self, item: SessionMetaItem, line_timestamp: Option<String>) -> Vec<AgentEvent> {
    if self.session_id.is_some() {
      return Vec::new();
    }

    let Some(session_id) = item.id.clone() else {
      return vec![unknown_event(
        None,
        Some("session_meta".to_string()),
        Some(json_value(item)),
        line_timestamp,
      )];
    };

    self.session_id = Some(session_id.clone());
    let timestamp = item.timestamp.clone().or(line_timestamp);
    let mut events = vec![AgentEvent::SessionStarted(SessionStarted {
      provider: Provider::Codex,
      session_id,
      cwd: item.cwd.clone(),
      timestamp: timestamp.clone(),
    })];

    if item.model_provider.is_some() {
      events.push(AgentEvent::ProviderChanged(ProviderChanged {
        provider: Provider::Codex,
        session_id: self.session_id.clone(),
        native_id: item.id,
        native_parent_id: item.parent_thread_id,
        model_provider: item.model_provider,
        model_id: None,
        thinking_level: None,
        timestamp,
      }));
    }

    events
  }
}

fn requires_thread_spawn_boundary(item: &SessionMetaItem) -> bool {
  item.source.as_ref().is_some_and(|source| match source {
    Value::Object(source) => source
      .get("subagent")
      .and_then(|subagent| subagent.get("thread_spawn"))
      .is_some(),
    Value::String(source) => source.starts_with("subagent_thread_spawn"),
    _ => false,
  }) || item.extra.get("subagent_source").and_then(Value::as_str) == Some("thread_spawn")
}

fn normalize_response_item(
  session_id: Option<String>,
  item: ResponseItem,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  match item {
    ResponseItem::Message(item) => normalize_message(session_id, item, timestamp),
    ResponseItem::AgentMessage(item) => normalize_agent_message(session_id, item, timestamp),
    ResponseItem::Reasoning(item) => normalize_reasoning(session_id, item, timestamp),
    ResponseItem::FunctionCall(item) => {
      let name = item
        .name
        .or(item.namespace)
        .unwrap_or_else(|| "function_call".to_string());
      let input = parse_json_string_or_value(item.arguments);
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: item.id,
        parent_id: None,
        tool_call_id: item.call_id,
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
    ResponseItem::FunctionCallOutput(item) => vec![tool_output_event(
      session_id,
      item.id,
      item.call_id,
      None,
      item.output,
      timestamp,
    )],
    ResponseItem::LocalShellCall(item) => {
      let input = item.action.unwrap_or(Value::Null);
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: item.id,
        parent_id: None,
        tool_call_id: item.call_id,
        tool_name: Some("local_shell".to_string()),
        tool_kind: ToolKind::Shell,
        summary: tool_summary_for_input("local_shell", &input),
        phase: Phase::Finished,
        input: Some(input),
        output: item.status.map(Value::String),
        is_error: None,
        timestamp,
      })]
    }
    ResponseItem::CustomToolCall(item) => {
      let name = item
        .name
        .or(item.namespace)
        .unwrap_or_else(|| "custom_tool".to_string());
      let input = parse_json_string_or_value(item.input);
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: item.id,
        parent_id: None,
        tool_call_id: item.call_id,
        tool_name: Some(name.clone()),
        tool_kind: tool_kind_for_name(&name),
        summary: tool_summary_for_input(&name, &input),
        phase: Phase::Finished,
        input: Some(input),
        output: item.status.map(Value::String),
        is_error: None,
        timestamp,
      })]
    }
    ResponseItem::CustomToolCallOutput(item) => vec![tool_output_event(
      session_id,
      item.id,
      item.call_id,
      item.name,
      item.output,
      timestamp,
    )],
    ResponseItem::ToolSearchCall(item) => {
      let input = parse_json_string_or_value(item.arguments);
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: item.id,
        parent_id: None,
        tool_call_id: item.call_id,
        tool_name: Some("tool_search".to_string()),
        tool_kind: ToolKind::Search,
        summary: tool_summary_for_input("tool_search", &input),
        phase: Phase::Finished,
        input: Some(input),
        output: item.status.map(Value::String),
        is_error: None,
        timestamp,
      })]
    }
    ResponseItem::ToolSearchOutput(item) => vec![tool_output_event(
      session_id,
      item.id,
      item.call_id,
      Some("tool_search".to_string()),
      json!({
        "status": item.status,
        "execution": item.execution,
        "tools": item.tools,
      }),
      timestamp,
    )],
    ResponseItem::WebSearchCall(item) => {
      let input = item.action.unwrap_or(Value::Null);
      vec![AgentEvent::ToolCall(ToolCallEvent {
        provider: Provider::Codex,
        session_id,
        message_id: item.id,
        parent_id: None,
        tool_call_id: None,
        tool_name: Some("web_search".to_string()),
        tool_kind: ToolKind::Search,
        summary: tool_summary_for_input("web_search", &input),
        phase: Phase::Finished,
        input: Some(input),
        output: item.status.map(Value::String),
        is_error: None,
        timestamp,
      })]
    }
    ResponseItem::ImageGenerationCall(item) => vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id,
      message_id: item.id,
      parent_id: None,
      tool_call_id: None,
      tool_name: Some("image_generation".to_string()),
      tool_kind: ToolKind::Unknown,
      summary: None,
      phase: Phase::Finished,
      input: item.revised_prompt.map(Value::String),
      output: item.result.map(Value::String),
      is_error: None,
      timestamp,
    })],
    ResponseItem::AdditionalTools(_)
    | ResponseItem::Compaction(_)
    | ResponseItem::CompactionTrigger(_)
    | ResponseItem::ContextCompaction(_) => Vec::new(),
    ResponseItem::Unknown(item) => vec![unknown_response_event(session_id, item, timestamp)],
  }
}

fn normalize_message(session_id: Option<String>, item: MessageItem, timestamp: Option<String>) -> Vec<AgentEvent> {
  let role = item.role.as_deref().unwrap_or("unknown");
  if role != "assistant" {
    return Vec::new();
  }
  let delivery = codex_message_delivery(item.phase.as_deref());

  let text = content_text(&item.content);
  if text.is_empty() {
    return vec![unknown_event(
      session_id,
      Some("response_item.message".to_string()),
      Some(json_value(item)),
      timestamp,
    )];
  }

  vec![AgentEvent::Message(MessageEvent {
    provenance: None,
    provider: Provider::Codex,
    session_id,
    message_id: item.id,
    parent_id: None,
    role: codex_role(role),
    delivery,
    phase: Phase::Finished,
    text,
    timestamp,
  })]
}

fn normalize_reasoning(session_id: Option<String>, item: ReasoningItem, timestamp: Option<String>) -> Vec<AgentEvent> {
  let text = content_text(item.content.as_deref().unwrap_or_default());
  let summary = content_text(&item.summary);
  if text.is_empty() && summary.is_empty() && item.encrypted_content.is_none() {
    return Vec::new();
  }

  vec![AgentEvent::Reasoning(ReasoningEvent {
    provenance: None,
    provider: Provider::Codex,
    session_id,
    message_id: item.id,
    parent_id: None,
    phase: Phase::Finished,
    text: present_text(text),
    summary: present_text(summary),
    encrypted_content: item.encrypted_content,
    signature: None,
    timestamp,
  })]
}

fn normalize_agent_message(
  session_id: Option<String>,
  item: AgentMessageItem,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  let native = json_value(&item);
  vec![AgentEvent::AgentActivity(AgentActivity {
    provider: Provider::Codex,
    session_id,
    event_id: item.id,
    actor_session_id: None,
    actor_agent_path: item.author,
    target_session_id: None,
    target_agent_path: item.recipient,
    kind: "messaged".to_string(),
    occurred_at_ms: None,
    native: Some(native),
    timestamp,
  })]
}

fn normalize_inter_agent_communication(
  session_id: Option<String>,
  item: InterAgentCommunicationItem,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  let native = json_value(&item);
  vec![AgentEvent::AgentActivity(AgentActivity {
    provider: Provider::Codex,
    session_id,
    event_id: item.id.clone(),
    actor_session_id: None,
    actor_agent_path: item.author.clone(),
    target_session_id: None,
    target_agent_path: item.recipient.clone(),
    kind: "messaged".to_string(),
    occurred_at_ms: None,
    native: Some(native),
    timestamp,
  })]
}

fn normalize_compacted(session_id: Option<String>, item: CompactedItem, timestamp: Option<String>) -> Vec<AgentEvent> {
  let Some(text) = item.message.filter(|message| !message.is_empty()) else {
    return Vec::new();
  };
  vec![message_event(
    session_id,
    None,
    Role::Assistant,
    Phase::Finished,
    text,
    timestamp,
  )]
}

fn normalize_event_message(
  session_id: Option<String>,
  item: EventMessage,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  let payload = item.native;
  match item.event_type.as_deref() {
    Some("task_started" | "turn_started" | "task_complete" | "turn_complete" | "token_count") => Vec::new(),
    Some("user_message") => string_field(&payload, "message")
      .map(|text| {
        vec![message_event(
          session_id,
          None,
          Role::User,
          Phase::Finished,
          text,
          timestamp,
        )]
      })
      .unwrap_or_default(),
    Some("agent_message") => Vec::new(),
    Some("agent_message_delta" | "agent_message_content_delta") => {
      normalize_message_delta(session_id, &payload, timestamp)
    }
    Some("agent_reasoning") => normalize_reasoning_event(session_id, &payload, "text", Phase::Finished, timestamp),
    Some("agent_reasoning_delta") => normalize_reasoning_event(session_id, &payload, "delta", Phase::Delta, timestamp),
    Some("agent_reasoning_raw_content") => {
      normalize_reasoning_event(session_id, &payload, "text", Phase::Finished, timestamp)
    }
    Some("agent_reasoning_raw_content_delta" | "reasoning_raw_content_delta") => {
      normalize_reasoning_event(session_id, &payload, "delta", Phase::Delta, timestamp)
    }
    Some("reasoning_content_delta") => {
      normalize_reasoning_event(session_id, &payload, "delta", Phase::Delta, timestamp)
    }
    Some("exec_command_begin") => normalize_exec_begin(session_id, &payload, timestamp),
    Some("exec_command_output_delta") => normalize_exec_delta(session_id, payload, timestamp),
    Some("exec_command_end") => normalize_exec_end(session_id, &payload, timestamp),
    Some("mcp_tool_call_begin") => normalize_mcp_begin(session_id, &payload, timestamp),
    Some("mcp_tool_call_end") => normalize_mcp_end(session_id, &payload, timestamp),
    Some("web_search_begin") => normalize_web_search_begin(session_id, &payload, timestamp),
    Some("web_search_end") => normalize_web_search_end(session_id, &payload, timestamp),
    Some("patch_apply_begin") => normalize_patch_begin(session_id, &payload, timestamp),
    Some("patch_apply_end") => normalize_patch_end(session_id, &payload, timestamp),
    Some("view_image_tool_call") => normalize_view_image(session_id, &payload, timestamp),
    Some("error" | "warning" | "guardian_warning" | "stream_error") => normalize_error(session_id, &payload, timestamp),
    Some("turn_aborted") => normalize_turn_aborted(session_id, &payload, timestamp),
    Some("thread_settings_applied") => {
      vec![session_settings_applied_event(session_id, &payload, timestamp)]
    }
    Some("thread_goal_updated") => vec![goal_updated_event(session_id, &payload, timestamp)],
    Some("sub_agent_activity") => normalize_sub_agent_activity(session_id, &payload, timestamp),
    _ => vec![unknown_event(
      session_id,
      item
        .event_type
        .map(|event_type| format!("event_msg.{event_type}"))
        .or_else(|| Some("event_msg".to_string())),
      Some(payload),
      timestamp,
    )],
  }
}

fn normalize_message_delta(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  let Some(text) = string_field(payload, "delta") else {
    return Vec::new();
  };
  vec![message_event(
    session_id,
    string_field(payload, "item_id"),
    Role::Assistant,
    Phase::Delta,
    text,
    timestamp,
  )]
}

fn normalize_reasoning_event(
  session_id: Option<String>,
  payload: &Value,
  field: &str,
  phase: Phase,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  let Some(text) = string_field(payload, field).filter(|text| !text.is_empty()) else {
    return Vec::new();
  };
  vec![AgentEvent::Reasoning(ReasoningEvent {
    provenance: None,
    provider: Provider::Codex,
    session_id,
    message_id: string_field(payload, "item_id"),
    parent_id: None,
    phase,
    text: Some(text),
    summary: None,
    encrypted_content: None,
    signature: None,
    timestamp,
  })]
}

fn normalize_exec_begin(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  let command = command_value(payload);
  vec![AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    tool_call_id: string_field(payload, "call_id"),
    tool_name: Some("exec_command".to_string()),
    tool_kind: ToolKind::Shell,
    summary: Some(ToolSummary::Shell {
      command: command_text(command.as_ref()),
      cwd: path_field(payload, "cwd"),
      exit_code: None,
    }),
    phase: Phase::Started,
    input: command,
    output: None,
    is_error: None,
    timestamp,
  })]
}

fn normalize_exec_delta(session_id: Option<String>, payload: Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  vec![AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    tool_call_id: string_field(&payload, "call_id"),
    tool_name: Some("exec_command".to_string()),
    tool_kind: ToolKind::Shell,
    summary: None,
    phase: Phase::Delta,
    input: None,
    output: Some(payload),
    is_error: None,
    timestamp,
  })]
}

fn normalize_exec_end(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  let exit_code = payload.get("exit_code").and_then(Value::as_i64);
  vec![AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    tool_call_id: string_field(payload, "call_id"),
    tool_name: Some("exec_command".to_string()),
    tool_kind: ToolKind::Shell,
    summary: Some(ToolSummary::Shell {
      command: command_text(command_value(payload).as_ref()),
      cwd: path_field(payload, "cwd"),
      exit_code,
    }),
    phase: Phase::Finished,
    input: None,
    output: Some(json!({
      "stdout": payload.get("stdout").cloned().unwrap_or(Value::Null),
      "stderr": payload.get("stderr").cloned().unwrap_or(Value::Null),
      "aggregated_output": payload.get("aggregated_output").cloned().unwrap_or(Value::Null),
      "formatted_output": payload.get("formatted_output").cloned().unwrap_or(Value::Null),
    })),
    is_error: exit_code.map(|code| code != 0),
    timestamp,
  })]
}

fn normalize_mcp_begin(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  let invocation = payload.get("invocation");
  let name = invocation.and_then(mcp_name);
  let input = invocation.and_then(|value| value.get("arguments")).cloned();
  vec![AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    tool_call_id: string_field(payload, "call_id"),
    tool_name: name.clone(),
    tool_kind: tool_kind_for_optional_name(name.as_deref()),
    summary: tool_summary_for_io(name.as_deref(), input.as_ref(), None),
    phase: Phase::Started,
    input,
    output: None,
    is_error: None,
    timestamp,
  })]
}

fn normalize_mcp_end(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  let name = payload.get("invocation").and_then(mcp_name);
  let output = payload.get("result").cloned().unwrap_or(Value::Null);
  vec![AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    tool_call_id: string_field(payload, "call_id"),
    tool_name: name.clone(),
    tool_kind: tool_kind_for_optional_name(name.as_deref()),
    summary: None,
    phase: Phase::Finished,
    input: None,
    output: Some(output.clone()),
    is_error: Some(mcp_result_is_error(&output)),
    timestamp,
  })]
}

fn normalize_web_search_begin(
  session_id: Option<String>,
  payload: &Value,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  vec![tool_lifecycle_event(
    session_id,
    string_field(payload, "call_id"),
    "web_search",
    ToolKind::Search,
    Phase::Started,
    None,
    None,
    timestamp,
  )]
}

fn normalize_web_search_end(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  let query = string_field(payload, "query");
  vec![AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    tool_call_id: string_field(payload, "call_id"),
    tool_name: Some("web_search".to_string()),
    tool_kind: ToolKind::Search,
    summary: Some(ToolSummary::Search { query: query.clone() }),
    phase: Phase::Finished,
    input: payload.get("action").cloned(),
    output: Some(json!({
      "query": query,
      "results": payload.get("results").cloned().unwrap_or(Value::Null),
    })),
    is_error: None,
    timestamp,
  })]
}

fn normalize_patch_begin(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  let changes = payload.get("changes").cloned().unwrap_or(Value::Null);
  vec![tool_lifecycle_event(
    session_id,
    string_field(payload, "call_id"),
    "apply_patch",
    ToolKind::FileEdit,
    Phase::Started,
    Some(changes.clone()),
    Some(patch_summary(&changes)),
    timestamp,
  )]
}

fn normalize_patch_end(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  let changes = payload.get("changes").cloned().unwrap_or(Value::Null);
  vec![AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    tool_call_id: string_field(payload, "call_id"),
    tool_name: Some("apply_patch".to_string()),
    tool_kind: ToolKind::FileEdit,
    summary: Some(patch_summary(&changes)),
    phase: Phase::Finished,
    input: None,
    output: Some(json!({
      "stdout": payload.get("stdout").cloned().unwrap_or(Value::Null),
      "stderr": payload.get("stderr").cloned().unwrap_or(Value::Null),
    })),
    is_error: payload.get("success").and_then(Value::as_bool).map(|success| !success),
    timestamp,
  })]
}

fn normalize_view_image(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  let path = path_field(payload, "path");
  vec![AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    tool_call_id: string_field(payload, "call_id"),
    tool_name: Some("view_image".to_string()),
    tool_kind: ToolKind::FileRead,
    summary: Some(ToolSummary::FileRead { path: path.clone() }),
    phase: Phase::Finished,
    input: path.map(|path| json!({ "path": path })),
    output: None,
    is_error: None,
    timestamp,
  })]
}

fn normalize_error(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  string_field(payload, "message")
    .map(|message| {
      vec![AgentEvent::Error(ErrorEvent {
        provider: Provider::Codex,
        session_id,
        message,
        timestamp,
      })]
    })
    .unwrap_or_default()
}

fn normalize_turn_aborted(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> Vec<AgentEvent> {
  let message = payload
    .get("reason")
    .map(display_json_value)
    .unwrap_or_else(|| "turn aborted".to_string());
  vec![AgentEvent::Error(ErrorEvent {
    provider: Provider::Codex,
    session_id,
    message,
    timestamp,
  })]
}

fn goal_updated_event(session_id: Option<String>, payload: &Value, timestamp: Option<String>) -> AgentEvent {
  AgentEvent::GoalUpdated(GoalUpdated {
    provider: Provider::Codex,
    session_id: string_field_any(payload, &["thread_id", "threadId"]).or(session_id),
    turn_id: string_field_any(payload, &["turn_id", "turnId"]),
    goal: payload.get("goal").cloned(),
    timestamp,
  })
}

fn normalize_sub_agent_activity(
  session_id: Option<String>,
  payload: &Value,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  let Some(kind) = string_field(payload, "kind") else {
    return vec![unknown_event(
      session_id,
      Some("event_msg.sub_agent_activity".to_string()),
      Some(payload.clone()),
      timestamp,
    )];
  };
  vec![AgentEvent::AgentActivity(AgentActivity {
    provider: Provider::Codex,
    session_id,
    event_id: string_field(payload, "event_id"),
    actor_session_id: None,
    actor_agent_path: None,
    target_session_id: string_field(payload, "agent_thread_id"),
    target_agent_path: string_field(payload, "agent_path"),
    kind,
    occurred_at_ms: payload.get("occurred_at_ms").and_then(Value::as_u64),
    native: Some(payload.clone()),
    timestamp,
  })]
}

fn session_settings_applied_event(
  session_id: Option<String>,
  payload: &Value,
  timestamp: Option<String>,
) -> AgentEvent {
  let settings = payload.get("thread_settings");
  AgentEvent::SessionSettingsApplied(SessionSettingsApplied {
    provider: Provider::Codex,
    session_id,
    model_provider: settings.and_then(|settings| string_field(settings, "model_provider_id")),
    model_id: settings.and_then(|settings| string_field(settings, "model")),
    service_tier: settings.and_then(|settings| string_field(settings, "service_tier")),
    cwd: settings.and_then(|settings| string_field(settings, "cwd")),
    reasoning_effort: settings.and_then(|settings| string_field(settings, "reasoning_effort")),
    reasoning_summary: settings.and_then(|settings| string_field(settings, "reasoning_summary")),
    personality: settings.and_then(|settings| string_field(settings, "personality")),
    collaboration_mode: settings
      .and_then(|settings| settings.get("collaboration_mode"))
      .and_then(|mode| string_field(mode, "mode")),
    approval_policy: settings.and_then(|settings| string_field(settings, "approval_policy")),
    approvals_reviewer: settings.and_then(|settings| string_field(settings, "approvals_reviewer")),
    active_permission_profile_id: settings
      .and_then(|settings| settings.get("active_permission_profile"))
      .and_then(|profile| string_field(profile, "id")),
    native: settings.cloned(),
    timestamp,
  })
}

fn tool_lifecycle_event(
  session_id: Option<String>,
  call_id: Option<String>,
  name: &str,
  kind: ToolKind,
  phase: Phase,
  input: Option<Value>,
  summary: Option<ToolSummary>,
  timestamp: Option<String>,
) -> AgentEvent {
  AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Codex,
    session_id,
    message_id: None,
    parent_id: None,
    tool_call_id: call_id,
    tool_name: Some(name.to_string()),
    tool_kind: kind,
    summary,
    phase,
    input,
    output: None,
    is_error: None,
    timestamp,
  })
}

fn tool_output_event(
  session_id: Option<String>,
  message_id: Option<String>,
  call_id: Option<String>,
  name: Option<String>,
  output: Value,
  timestamp: Option<String>,
) -> AgentEvent {
  AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::Codex,
    session_id,
    message_id,
    parent_id: None,
    tool_call_id: call_id,
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

fn message_event(
  session_id: Option<String>,
  message_id: Option<String>,
  role: Role,
  phase: Phase,
  text: String,
  timestamp: Option<String>,
) -> AgentEvent {
  AgentEvent::Message(MessageEvent {
    provenance: None,
    provider: Provider::Codex,
    session_id,
    message_id,
    parent_id: None,
    role,
    delivery: MessageDelivery::Unspecified,
    phase,
    text,
    timestamp,
  })
}

fn unknown_rollout_event(session_id: Option<String>, item: UnknownItem, timestamp: Option<String>) -> AgentEvent {
  unknown_event(session_id, item.native_type, Some(item.payload), timestamp)
}

fn unknown_response_event(session_id: Option<String>, item: UnknownItem, timestamp: Option<String>) -> AgentEvent {
  let native_type = item
    .native_type
    .map(|native_type| format!("response_item.{native_type}"))
    .or_else(|| Some("response_item".to_string()));
  unknown_event(session_id, native_type, Some(item.payload), timestamp)
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

fn content_text(content: &[ContentItem]) -> String {
  content
    .iter()
    .filter_map(|item| item.text.as_deref())
    .filter(|text| !text.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
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

fn codex_message_delivery(phase: Option<&str>) -> MessageDelivery {
  match phase {
    Some("commentary") => MessageDelivery::Commentary,
    Some("final") | Some("final_answer") => MessageDelivery::Final,
    _ => MessageDelivery::Unspecified,
  }
}

fn parse_json_string_or_value(value: Value) -> Value {
  match value {
    Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
    value => value,
  }
}

fn present_text(text: String) -> Option<String> {
  (!text.is_empty()).then_some(text)
}

fn string_field(value: &Value, field: &str) -> Option<String> {
  value
    .get(field)
    .and_then(Value::as_str)
    .filter(|value| !value.is_empty())
    .map(str::to_string)
}

fn string_field_any(value: &Value, fields: &[&str]) -> Option<String> {
  fields.iter().find_map(|field| string_field(value, field))
}

fn path_field(value: &Value, field: &str) -> Option<String> {
  let path = value.get(field)?;
  path
    .as_str()
    .map(str::to_string)
    .or_else(|| string_field_any(path, &["path", "uri"]))
}

fn command_value(payload: &Value) -> Option<Value> {
  payload.get("command").cloned().filter(|command| !command.is_null())
}

fn command_text(command: Option<&Value>) -> Option<String> {
  match command {
    Some(Value::String(command)) => Some(command.clone()),
    Some(Value::Array(parts)) => {
      let command = parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ");
      (!command.is_empty()).then_some(command)
    }
    _ => None,
  }
}

fn mcp_name(invocation: &Value) -> Option<String> {
  let server = string_field(invocation, "server")?;
  let tool = string_field(invocation, "tool")?;
  Some(format!("{server}.{tool}"))
}

fn mcp_result_is_error(result: &Value) -> bool {
  if result.get("Err").is_some() || result.get("err").is_some() {
    return true;
  }
  result
    .get("Ok")
    .or_else(|| result.get("ok"))
    .unwrap_or(result)
    .get("is_error")
    .and_then(Value::as_bool)
    .unwrap_or(false)
}

fn display_json_value(value: &Value) -> String {
  value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string())
}

fn json_value(value: impl serde::Serialize) -> Value {
  serde_json::to_value(value).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
  use super::*;

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

  fn normalize_historical_fixture(input: &str) -> (Vec<AgentEvent>, SessionHistoryStatus) {
    let mut normalizer = CodexNormalizer::new_historical();
    let events = input
      .lines()
      .filter(|line| !line.trim().is_empty())
      .flat_map(|line| {
        let line: CodexLine = serde_json::from_str(line).expect("fixture line should parse");
        normalizer.normalize(line)
      })
      .collect();
    (events, normalizer.history_status())
  }

  #[test]
  fn normalizes_basic_fixture_events() {
    let events = normalize_fixture(include_str!("../fixtures/basic_session.jsonl"));

    assert_eq!(events.len(), 11);
    assert!(matches!(&events[0], AgentEvent::SessionStarted(event) if event.session_id == "session-fixture"));
    assert!(
      matches!(&events[1], AgentEvent::ProviderChanged(event) if event.model_provider.as_deref() == Some("openai"))
    );
    assert!(
      matches!(&events[2], AgentEvent::Message(event) if matches!(event.role, Role::User) && event.text == "build a tiny test")
    );
    assert!(
      matches!(&events[3], AgentEvent::Message(event) if event.message_id.as_deref() == Some("msg-assistant") && event.text == "done")
    );
    assert!(matches!(&events[4], AgentEvent::Reasoning(event) if event.summary.as_deref() == Some("checked files")));
    assert!(
      matches!(&events[5], AgentEvent::ToolCall(event) if matches!(event.phase, Phase::Started) && matches!(event.tool_kind, ToolKind::Shell))
    );
    assert!(
      matches!(&events[6], AgentEvent::ToolCall(event) if matches!(event.phase, Phase::Started) && matches!(event.tool_kind, ToolKind::FileEdit))
    );
    assert!(matches!(&events[7], AgentEvent::GoalUpdated(event) if event.turn_id.as_deref() == Some("turn-1")));
    assert!(
      matches!(&events[8], AgentEvent::SessionSettingsApplied(event) if event.model_id.as_deref() == Some("gpt-5"))
    );
    assert!(
      matches!(&events[9], AgentEvent::AgentActivity(event) if event.target_agent_path.as_deref() == Some("/root"))
    );
    assert!(
      matches!(&events[10], AgentEvent::Unknown(event) if event.native_type.as_deref() == Some("event_msg.new_native_event"))
    );
  }

  #[test]
  fn normalizes_current_protocol_records_without_unknown_noise() {
    let events = normalize_fixture(
      r#"{"type":"session_meta","payload":{"id":"session-1","cwd":"/tmp/project"}}
{"type":"response_item","payload":{"type":"custom_tool_call_output","id":"out-1","call_id":"call-1","name":"exec","output":[{"type":"input_text","text":"done"}]}}
{"type":"response_item","payload":{"type":"agent_message","id":"amsg-1","author":"/root","recipient":"/root/reviewer","content":[{"type":"input_text","text":"review this"}]}}
{"type":"inter_agent_communication_metadata","payload":{"trigger_turn":true}}
{"type":"world_state","payload":{"full":false,"state":{"large":"value"}}}
{"type":"turn_context","payload":{"turn_id":"turn-1","effort":"ultra"}}"#,
    );

    assert_eq!(events.len(), 3);
    let AgentEvent::ToolCall(output) = &events[1] else {
      panic!("expected custom tool output");
    };
    assert_eq!(output.message_id.as_deref(), Some("out-1"));
    assert!(output.output.as_ref().is_some_and(Value::is_array));
    let AgentEvent::AgentActivity(activity) = &events[2] else {
      panic!("expected agent activity");
    };
    assert_eq!(activity.actor_agent_path.as_deref(), Some("/root"));
    assert_eq!(activity.target_agent_path.as_deref(), Some("/root/reviewer"));
    assert_eq!(activity.kind, "messaged");
  }

  #[test]
  fn keeps_unknown_response_identity_and_payload() {
    let events = normalize_fixture(
      r#"{"type":"session_meta","payload":{"id":"session-1"}}
{"type":"response_item","payload":{"type":"future_response","id":"future-1","answer":42}}"#,
    );

    let AgentEvent::Unknown(event) = &events[1] else {
      panic!("expected unknown event");
    };
    assert_eq!(event.native_type.as_deref(), Some("response_item.future_response"));
    assert_eq!(
      event.native.as_ref().and_then(|value| value.get("answer")),
      Some(&json!(42))
    );
  }

  #[test]
  fn preserves_assistant_message_delivery() {
    let events = normalize_fixture(
      r#"{"type":"session_meta","payload":{"id":"session-1"}}
{"type":"response_item","payload":{"type":"message","id":"commentary","role":"assistant","content":[{"type":"output_text","text":"working"}],"phase":"commentary"}}
{"type":"response_item","payload":{"type":"message","id":"final","role":"assistant","content":[{"type":"output_text","text":"done"}],"phase":"final"}}
{"type":"response_item","payload":{"type":"message","id":"final-answer","role":"assistant","content":[{"type":"output_text","text":"current done"}],"phase":"final_answer"}}"#,
    );

    assert!(matches!(&events[1], AgentEvent::Message(event) if matches!(event.delivery, MessageDelivery::Commentary)));
    assert!(matches!(&events[2], AgentEvent::Message(event) if matches!(event.delivery, MessageDelivery::Final)));
    assert!(matches!(&events[3], AgentEvent::Message(event) if matches!(event.delivery, MessageDelivery::Final)));
  }

  #[test]
  fn keeps_first_session_header_when_parent_history_is_copied() {
    let events = normalize_fixture(
      r#"{"timestamp":"2026-07-24T17:52:40Z","type":"session_meta","payload":{"id":"child-session","parent_thread_id":"root-session","timestamp":"2026-07-24T17:52:40Z","cwd":"/tmp/project","agent_path":"/root/researcher"}}
{"timestamp":"2026-07-15T10:00:00Z","type":"session_meta","payload":{"id":"root-session","timestamp":"2026-07-15T10:00:00Z","cwd":"/tmp/project"}}
{"timestamp":"2026-07-24T17:54:07Z","type":"event_msg","payload":{"type":"sub_agent_activity","event_id":"call-agent","occurred_at_ms":1784915647361,"agent_thread_id":"root-session","agent_path":"/root","kind":"interacted"}}"#,
    );

    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0], AgentEvent::SessionStarted(event) if event.session_id == "child-session"));
    assert!(
      matches!(&events[1], AgentEvent::AgentActivity(event) if event.target_session_id.as_deref() == Some("root-session"))
    );
  }

  #[test]
  fn default_normalizer_keeps_streaming_subagent_events_without_a_historical_boundary() {
    let events = normalize_fixture(
      r#"{"type":"session_meta","payload":{"id":"child-session","parent_thread_id":"root-session"}}
{"type":"response_item","payload":{"type":"message","id":"child-message","role":"assistant","content":[{"type":"output_text","text":"live child result"}],"phase":"final"}}"#,
    );

    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0], AgentEvent::SessionStarted(event) if event.session_id == "child-session"));
    assert!(matches!(&events[1], AgentEvent::Message(event) if event.text == "live child result"));
  }

  #[test]
  fn historical_normalizer_starts_subagent_body_at_a_legacy_trigger_boundary() {
    let (events, status) = normalize_historical_fixture(
      r#"{"type":"session_meta","payload":{"id":"child-session","parent_thread_id":"root-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"root-session","depth":1,"agent_path":"/root/child"}}}}}
{"type":"event_msg","payload":{"type":"user_message","message":"copied parent request"}}
{"type":"inter_agent_communication","payload":{"id":"queued","author":"/root","recipient":"/root/child","content":"queued","trigger_turn":false}}
{"type":"inter_agent_communication","payload":{"id":"trigger","author":"/root","recipient":"/root/child","content":"start","trigger_turn":true}}
{"type":"response_item","payload":{"type":"message","id":"child-message","role":"assistant","content":[{"type":"output_text","text":"owned child result"}],"phase":"final"}}"#,
    );

    assert_eq!(status, SessionHistoryStatus::FilteredSubagent);
    assert_eq!(events.len(), 3);
    assert!(matches!(&events[0], AgentEvent::SessionStarted(event) if event.session_id == "child-session"));
    assert!(matches!(&events[1], AgentEvent::AgentActivity(event) if event.event_id.as_deref() == Some("trigger")));
    assert!(matches!(&events[2], AgentEvent::Message(event) if event.text == "owned child result"));
  }
}
