use crate::event::{
  OpenCodeMessage, OpenCodeMessageRow, OpenCodePart, OpenCodePartRow, OpenCodeRole, OpenCodeSessionRow,
  OpenCodeToolState,
};
use serde_json::{Value, json};
use tokn_session_core::{
  AgentEvent, ErrorEvent, MessageEvent, Phase, Provider, ProviderChanged, ReasoningEvent, Role, SessionStarted,
  ToolCallEvent, UnknownEvent, tool_kind_for_optional_name, tool_summary_for_io,
};

pub struct OpenCodeNormalizer {
  session_id: String,
  current_provider: Option<String>,
  current_model: Option<String>,
}

impl OpenCodeNormalizer {
  pub fn new(session_id: String) -> Self {
    Self {
      session_id,
      current_provider: None,
      current_model: None,
    }
  }

  pub fn normalize_session(&mut self, row: &OpenCodeSessionRow) -> Vec<AgentEvent> {
    let mut events = vec![AgentEvent::SessionStarted(SessionStarted {
      provider: Provider::OpenCode,
      session_id: row.id.clone(),
      cwd: row.directory.clone(),
      timestamp: timestamp(row.time_created),
    })];

    if let Some(model) = &row.model {
      let provider_id = string_field(model, "providerID");
      let model_id = string_field(model, "id");
      if provider_id.is_some() || model_id.is_some() {
        self.current_provider = provider_id.clone();
        self.current_model = model_id.clone();
        events.push(AgentEvent::ProviderChanged(ProviderChanged {
          provider: Provider::OpenCode,
          session_id: Some(row.id.clone()),
          native_id: None,
          native_parent_id: None,
          model_provider: provider_id,
          model_id,
          thinking_level: None,
          timestamp: timestamp(row.time_created),
        }));
      }
    }

    events
  }

  pub fn normalize_message(&mut self, row: OpenCodeMessageRow) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    events.extend(self.normalize_model(&row.data, row.time_created));

    match row.data.role {
      OpenCodeRole::User | OpenCodeRole::System => {
        let text = row
          .parts
          .into_iter()
          .filter_map(text_part_text)
          .collect::<Vec<_>>()
          .join("\n");
        if !text.is_empty() {
          events.push(AgentEvent::Message(MessageEvent {
            provider: Provider::OpenCode,
            session_id: Some(self.session_id.clone()),
            message_id: Some(row.id),
            parent_id: row.data.parent_id,
            role: open_code_role(row.data.role),
            phase: Phase::Finished,
            text,
            timestamp: timestamp(row.time_created),
          }));
        }
      }
      OpenCodeRole::Assistant => {
        if let Some(error) = row.data.error {
          events.push(AgentEvent::Error(ErrorEvent {
            provider: Provider::OpenCode,
            session_id: Some(self.session_id.clone()),
            message: error_message(error),
            timestamp: timestamp(row.time_created),
          }));
        }
        for part in row.parts {
          events.extend(self.normalize_assistant_part(&row.id, &row.data.parent_id, part));
        }
      }
      OpenCodeRole::Tool | OpenCodeRole::Unknown => {
        events.push(unknown_event(
          Some(self.session_id.clone()),
          Some(format!("message.role.{:?}", row.data.role).to_lowercase()),
          None,
          timestamp(row.time_created),
        ));
      }
    }

    events
  }

  fn normalize_model(&mut self, message: &OpenCodeMessage, time_created: Option<i64>) -> Vec<AgentEvent> {
    let (provider_id, model_id) = match message.role {
      OpenCodeRole::Assistant => (message.provider_id.clone(), message.model_id.clone()),
      _ => (
        message.model.as_ref().and_then(|model| model.provider_id.clone()),
        message.model.as_ref().and_then(|model| model.model_id.clone()),
      ),
    };

    if provider_id == self.current_provider && model_id == self.current_model {
      return Vec::new();
    }
    if provider_id.is_none() && model_id.is_none() {
      return Vec::new();
    }

    self.current_provider = provider_id.clone();
    self.current_model = model_id.clone();
    vec![AgentEvent::ProviderChanged(ProviderChanged {
      provider: Provider::OpenCode,
      session_id: Some(self.session_id.clone()),
      native_id: None,
      native_parent_id: None,
      model_provider: provider_id,
      model_id,
      thinking_level: None,
      timestamp: timestamp(time_created),
    })]
  }

  fn normalize_assistant_part(
    &self,
    message_id: &str,
    parent_id: &Option<String>,
    part: OpenCodePartRow,
  ) -> Vec<AgentEvent> {
    match part.data {
      OpenCodePart::Text { text } => vec![AgentEvent::Message(MessageEvent {
        provider: Provider::OpenCode,
        session_id: Some(self.session_id.clone()),
        message_id: Some(message_id.to_string()),
        parent_id: parent_id.clone(),
        role: Role::Assistant,
        phase: Phase::Finished,
        text,
        timestamp: timestamp(part.time_created),
      })],
      OpenCodePart::Reasoning { text } => vec![AgentEvent::Reasoning(ReasoningEvent {
        provider: Provider::OpenCode,
        session_id: Some(self.session_id.clone()),
        message_id: Some(message_id.to_string()),
        parent_id: parent_id.clone(),
        phase: Phase::Finished,
        text: Some(text),
        summary: None,
        encrypted_content: None,
        signature: None,
        timestamp: timestamp(part.time_created),
      })],
      OpenCodePart::Tool { call_id, tool, state } => vec![tool_event(
        self.session_id.clone(),
        message_id.to_string(),
        parent_id.clone(),
        call_id,
        tool,
        state,
        part.time_created,
      )],
      OpenCodePart::StepStart {} | OpenCodePart::StepFinish { .. } => Vec::new(),
      OpenCodePart::Unknown(value) => vec![unknown_event(
        Some(self.session_id.clone()),
        unknown_type("part", &value),
        Some(value),
        timestamp(part.time_created),
      )],
    }
  }
}

fn text_part_text(part: OpenCodePartRow) -> Option<String> {
  match part.data {
    OpenCodePart::Text { text } => Some(text),
    _ => None,
  }
}

pub(crate) fn tool_event(
  session_id: String,
  message_id: String,
  parent_id: Option<String>,
  call_id: Option<String>,
  tool: Option<String>,
  state: OpenCodeToolState,
  time_created: Option<i64>,
) -> AgentEvent {
  let (phase, input, output, is_error) = match state {
    OpenCodeToolState::Pending { input, raw } => {
      let input = match (input, raw) {
        (Some(input), Some(raw)) => Some(json!({ "input": input, "raw": raw })),
        (Some(input), None) => Some(input),
        (None, Some(raw)) => Some(json!({ "raw": raw })),
        (None, None) => None,
      };
      (Phase::Started, input, None, None)
    }
    OpenCodeToolState::Running { input, title, metadata } => (
      Phase::Updated,
      input,
      present_output(title, None, metadata),
      Some(false),
    ),
    OpenCodeToolState::Completed {
      input,
      output,
      title,
      metadata,
    } => {
      let is_error = metadata_exit_is_error(metadata.as_ref());
      (
        Phase::Finished,
        input,
        present_output(title, output.map(Value::String), metadata),
        Some(is_error),
      )
    }
    OpenCodeToolState::Error { input, error, metadata } => (
      Phase::Finished,
      input,
      present_output(None, error.map(Value::String), metadata),
      Some(true),
    ),
    OpenCodeToolState::Unknown(value) => (Phase::Updated, None, Some(value), None),
  };

  AgentEvent::ToolCall(ToolCallEvent {
    provider: Provider::OpenCode,
    session_id: Some(session_id),
    message_id: Some(message_id),
    parent_id,
    tool_call_id: call_id,
    tool_kind: tool_kind_for_optional_name(tool.as_deref()),
    summary: tool_summary_for_io(tool.as_deref(), input.as_ref(), output.as_ref()),
    tool_name: tool,
    phase,
    input,
    output,
    is_error,
    timestamp: timestamp(time_created),
  })
}

fn present_output(title: Option<String>, output: Option<Value>, metadata: Option<Value>) -> Option<Value> {
  if title.is_none() && output.is_none() && metadata.is_none() {
    return None;
  }
  Some(json!({
    "title": title,
    "output": output,
    "metadata": metadata,
  }))
}

fn metadata_exit_is_error(metadata: Option<&Value>) -> bool {
  metadata
    .and_then(|metadata| metadata.get("exit"))
    .and_then(Value::as_i64)
    .is_some_and(|exit| exit != 0)
}

fn open_code_role(role: OpenCodeRole) -> Role {
  match role {
    OpenCodeRole::User => Role::User,
    OpenCodeRole::Assistant => Role::Assistant,
    OpenCodeRole::System => Role::System,
    OpenCodeRole::Tool => Role::Tool,
    OpenCodeRole::Unknown => Role::Unknown,
  }
}

fn error_message(value: Value) -> String {
  value
    .get("data")
    .and_then(|data| data.get("message"))
    .and_then(Value::as_str)
    .or_else(|| value.get("message").and_then(Value::as_str))
    .map(str::to_string)
    .unwrap_or_else(|| value.to_string())
}

pub(crate) fn timestamp(value: Option<i64>) -> Option<String> {
  value.map(|value| value.to_string())
}

fn string_field(value: &Value, field: &str) -> Option<String> {
  value.get(field).and_then(Value::as_str).map(str::to_string)
}

fn unknown_type(prefix: &str, value: &Value) -> Option<String> {
  let suffix = value.get("type").and_then(Value::as_str).unwrap_or("unknown");
  Some(format!("{prefix}.{suffix}"))
}

fn unknown_event(
  session_id: Option<String>,
  native_type: Option<String>,
  native: Option<Value>,
  timestamp: Option<String>,
) -> AgentEvent {
  AgentEvent::Unknown(UnknownEvent {
    provider: Provider::OpenCode,
    session_id,
    native_type,
    native,
    timestamp,
  })
}
