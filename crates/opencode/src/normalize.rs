use crate::row::{OpenCodeMessageRow, OpenCodePartRow, OpenCodeSessionRow};
use serde_json::{Value, json};
use tokn_opencode_protocol::v1::{MessageItem, PartItem, ToolState, ToolStateItem};
use tokn_session_core::{
  AgentEvent, ErrorEvent, MessageDelivery, MessageEvent, Phase, Provider, ProviderChanged, ReasoningEvent, Role,
  SessionStarted, ToolCallEvent, UnknownEvent, tool_kind_for_optional_name, tool_summary_for_io,
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

    if let Some(model) = &row.model
      && (model.provider_id.is_some() || model.id.is_some())
    {
      self.current_provider = model.provider_id.clone();
      self.current_model = model.id.clone();
      events.push(AgentEvent::ProviderChanged(ProviderChanged {
        provider: Provider::OpenCode,
        session_id: Some(row.id.clone()),
        native_id: None,
        native_parent_id: None,
        model_provider: model.provider_id.clone(),
        model_id: model.id.clone(),
        thinking_level: None,
        timestamp: timestamp(row.time_created),
      }));
    }

    events
  }

  pub fn normalize_message(&mut self, row: OpenCodeMessageRow) -> Vec<AgentEvent> {
    let OpenCodeMessageRow {
      id,
      time_created,
      data,
      parts,
    } = row;
    let (message, native) = data.into_parts();
    let mut events = self.normalize_model(&message, &native, time_created);
    let parent_id = string_field(&native, "parentID");

    match message {
      MessageItem::User(_) => {
        events.extend(self.text_message(id, parent_id, Role::User, time_created, parts));
      }
      MessageItem::Assistant(message) => {
        if let Some(error) = message.error {
          events.push(AgentEvent::Error(ErrorEvent {
            provider: Provider::OpenCode,
            session_id: Some(self.session_id.clone()),
            message: error_message(error),
            timestamp: timestamp(time_created),
          }));
        }
        for part in parts {
          events.extend(self.normalize_assistant_part(&id, &message.parent_id, part));
        }
      }
      MessageItem::Unknown(item) if item.native_type.as_deref() == Some("system") => {
        events.extend(self.text_message(id, parent_id, Role::System, time_created, parts));
      }
      MessageItem::Unknown(item) if item.native_type.as_deref() == Some("user") => {
        events.extend(self.text_message(id, parent_id, Role::User, time_created, parts));
        events.push(self.unknown_message(item, time_created));
      }
      MessageItem::Unknown(item) if item.native_type.as_deref() == Some("assistant") => {
        if let Some(error) = item.native.get("error").filter(|error| !error.is_null()).cloned() {
          events.push(AgentEvent::Error(ErrorEvent {
            provider: Provider::OpenCode,
            session_id: Some(self.session_id.clone()),
            message: error_message(error),
            timestamp: timestamp(time_created),
          }));
        }
        for part in parts {
          events.extend(self.normalize_assistant_part(&id, &parent_id, part));
        }
        events.push(self.unknown_message(item, time_created));
      }
      MessageItem::Unknown(item) => {
        events.push(self.unknown_message(item, time_created));
      }
    }

    events
  }

  fn unknown_message(&self, item: tokn_opencode_protocol::UnknownItem, time_created: Option<i64>) -> AgentEvent {
    unknown_event(
      Some(self.session_id.clone()),
      item
        .native_type
        .as_deref()
        .map(|role| format!("message.role.{role}"))
        .or_else(|| Some("message.role.unknown".to_string())),
      Some(item.native),
      timestamp(time_created),
    )
  }

  fn text_message(
    &self,
    message_id: String,
    parent_id: Option<String>,
    role: Role,
    time_created: Option<i64>,
    parts: Vec<OpenCodePartRow>,
  ) -> Vec<AgentEvent> {
    let mut texts = Vec::new();
    let mut unknowns = Vec::new();
    for part in parts {
      let (item, native) = part.data.into_parts();
      match item {
        PartItem::Text(part) => texts.push(part.text),
        item => unknowns.push(unknown_event(
          Some(self.session_id.clone()),
          Some(format!("part.{}", item.native_type().unwrap_or("unknown"))),
          Some(native),
          timestamp(part.time_created),
        )),
      }
    }

    let text = texts.join("\n");
    let mut events = Vec::with_capacity(unknowns.len() + 1);
    if !text.is_empty() {
      events.push(AgentEvent::Message(MessageEvent {
        provider: Provider::OpenCode,
        session_id: Some(self.session_id.clone()),
        message_id: Some(message_id),
        parent_id,
        role,
        delivery: MessageDelivery::Unspecified,
        phase: Phase::Finished,
        text,
        timestamp: timestamp(time_created),
      }));
    }
    events.extend(unknowns);
    events
  }

  fn normalize_model(&mut self, message: &MessageItem, native: &Value, time_created: Option<i64>) -> Vec<AgentEvent> {
    let (provider_id, model_id) = match message {
      MessageItem::User(message) => (
        message.model.as_ref().and_then(|model| model.provider_id.clone()),
        message.model.as_ref().and_then(|model| model.model_id.clone()),
      ),
      MessageItem::Assistant(message) => (message.provider_id.clone(), message.model_id.clone()),
      MessageItem::Unknown(_) => model_from_native(native),
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
    let OpenCodePartRow {
      id: part_id,
      time_created,
      data,
    } = part;
    let (item, native) = data.into_parts();
    let native_type = item.native_type().map(str::to_string);

    match item {
      PartItem::Text(part) => vec![AgentEvent::Message(MessageEvent {
        provider: Provider::OpenCode,
        session_id: Some(self.session_id.clone()),
        message_id: Some(message_id.to_string()),
        parent_id: parent_id.clone(),
        role: Role::Assistant,
        delivery: MessageDelivery::Final,
        phase: Phase::Finished,
        text: part.text,
        timestamp: timestamp(time_created),
      })],
      PartItem::Reasoning(part) => vec![AgentEvent::Reasoning(ReasoningEvent {
        provider: Provider::OpenCode,
        session_id: Some(self.session_id.clone()),
        message_id: Some(message_id.to_string()),
        parent_id: parent_id.clone(),
        phase: Phase::Finished,
        text: Some(part.text),
        summary: None,
        encrypted_content: None,
        signature: None,
        timestamp: timestamp(time_created),
      })],
      PartItem::Tool(part) => vec![tool_event(
        self.session_id.clone(),
        message_id.to_string(),
        parent_id.clone(),
        part.call_id.or(part.identity.id).or(Some(part_id)),
        part.tool,
        part.state,
        time_created,
      )],
      PartItem::StepStart(_) | PartItem::StepFinish(_) => Vec::new(),
      _ => vec![unknown_event(
        Some(self.session_id.clone()),
        Some(format!("part.{}", native_type.as_deref().unwrap_or("unknown"))),
        Some(native),
        timestamp(time_created),
      )],
    }
  }
}

pub(crate) fn tool_event(
  session_id: String,
  message_id: String,
  parent_id: Option<String>,
  call_id: Option<String>,
  tool: Option<String>,
  state: ToolState,
  time_created: Option<i64>,
) -> AgentEvent {
  let (state, native) = state.into_parts();
  let (phase, input, output, is_error) = match state {
    ToolStateItem::Pending(state) => {
      let input = match (state.input, state.raw) {
        (Some(input), Some(raw)) => Some(json!({ "input": input, "raw": raw })),
        (Some(input), None) => Some(input),
        (None, Some(raw)) => Some(json!({ "raw": raw })),
        (None, None) => None,
      };
      (Phase::Started, input, None, None)
    }
    ToolStateItem::Running(state) => (
      Phase::Updated,
      state.input,
      present_output(state.title, None, state.metadata, None),
      Some(false),
    ),
    ToolStateItem::Completed(state) => {
      let is_error = metadata_exit_is_error(state.metadata.as_ref());
      (
        Phase::Finished,
        state.input,
        present_output(state.title, state.output, state.metadata, None),
        Some(is_error),
      )
    }
    ToolStateItem::Error(state) => (
      Phase::Finished,
      state.input,
      present_output(None, state.error.map(Value::String), state.metadata, state.raw),
      Some(true),
    ),
    ToolStateItem::Unknown(_) => (Phase::Updated, None, Some(native), None),
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

fn present_output(
  title: Option<String>,
  output: Option<Value>,
  metadata: Option<Value>,
  raw: Option<String>,
) -> Option<Value> {
  if title.is_none() && output.is_none() && metadata.is_none() && raw.is_none() {
    return None;
  }

  let mut value = json!({
    "title": title,
    "output": output,
    "metadata": metadata,
  });
  if let (Some(object), Some(raw)) = (value.as_object_mut(), raw) {
    object.insert("raw".to_string(), Value::String(raw));
  }
  Some(value)
}

fn metadata_exit_is_error(metadata: Option<&Value>) -> bool {
  metadata
    .and_then(|metadata| metadata.get("exit"))
    .and_then(Value::as_i64)
    .is_some_and(|exit| exit != 0)
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

fn model_from_native(value: &Value) -> (Option<String>, Option<String>) {
  let direct_provider = string_field(value, "providerID");
  let direct_model = string_field(value, "modelID");
  let model = value.get("model");
  let nested_provider = model.and_then(|model| string_field(model, "providerID"));
  let nested_model = model.and_then(|model| string_field(model, "modelID").or_else(|| string_field(model, "id")));
  (direct_provider.or(nested_provider), direct_model.or(nested_model))
}

fn string_field(value: &Value, field: &str) -> Option<String> {
  value.get(field).and_then(Value::as_str).map(str::to_string)
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

#[cfg(test)]
mod tests {
  use serde_json::{Value, json};
  use tokn_opencode_protocol::v1::{MessageData, PartData};
  use tokn_session_core::{AgentEvent, MessageDelivery, Phase, Role};

  use super::OpenCodeNormalizer;
  use crate::row::{OpenCodeMessageRow, OpenCodePartRow};

  #[test]
  fn normalizes_known_persisted_messages_and_parts() {
    let mut normalizer = OpenCodeNormalizer::new("ses_1".to_string());
    let user = message_row(
      "msg_user",
      json!({
        "role": "user",
        "model": {
          "providerID": "openai",
          "modelID": "gpt-5"
        }
      }),
      vec![part_row("prt_user", json!({"type": "text", "text": "hello"}))],
    );
    let user_events = normalizer.normalize_message(user);

    assert!(matches!(&user_events[0], AgentEvent::ProviderChanged(event)
      if event.model_provider.as_deref() == Some("openai")
        && event.model_id.as_deref() == Some("gpt-5")));
    assert!(matches!(&user_events[1], AgentEvent::Message(event)
      if matches!(event.role, Role::User) && event.text == "hello"));

    let assistant = message_row(
      "msg_assistant",
      json!({
        "role": "assistant",
        "parentID": "msg_user",
        "providerID": "openai",
        "modelID": "gpt-5"
      }),
      vec![
        part_row("prt_reasoning", json!({"type": "reasoning", "text": "checking"})),
        part_row("prt_text", json!({"type": "text", "text": "done"})),
        part_row(
          "prt_tool",
          json!({
            "type": "tool",
            "callID": "call_1",
            "tool": "bash",
            "state": {
              "status": "completed",
              "input": {"command": "false"},
              "output": "",
              "metadata": {"exit": 1}
            }
          }),
        ),
      ],
    );
    let assistant_events = normalizer.normalize_message(assistant);

    assert!(matches!(&assistant_events[0], AgentEvent::Reasoning(event)
      if event.text.as_deref() == Some("checking")));
    assert!(matches!(&assistant_events[1], AgentEvent::Message(event)
      if matches!(event.role, Role::Assistant)
        && matches!(event.delivery, MessageDelivery::Final)
        && event.text == "done"));
    assert!(matches!(&assistant_events[2], AgentEvent::ToolCall(event)
      if event.tool_call_id.as_deref() == Some("call_1")
        && matches!(event.phase, Phase::Finished)
        && event.is_error == Some(true)));
  }

  #[test]
  fn preserves_unknown_and_malformed_native_payloads() {
    let mut normalizer = OpenCodeNormalizer::new("ses_1".to_string());
    let unknown = message_row(
      "msg_unknown",
      json!({
        "role": "future-role",
        "payload": {"answer": 42}
      }),
      Vec::new(),
    );
    let events = normalizer.normalize_message(unknown);
    assert!(matches!(&events[0], AgentEvent::Unknown(event)
      if event.native_type.as_deref() == Some("message.role.future-role")
        && event.native.as_ref().and_then(|native| native.pointer("/payload/answer")) == Some(&json!(42))));

    let malformed_part = message_row(
      "msg_assistant",
      json!({"role": "assistant"}),
      vec![part_row("prt_bad", json!({"type": "text", "text": 42}))],
    );
    let events = normalizer.normalize_message(malformed_part);
    assert!(matches!(&events[0], AgentEvent::Unknown(event)
      if event.native_type.as_deref() == Some("part.text")
        && event.native.as_ref().and_then(|native| native.get("text")) == Some(&json!(42))));

    let user_with_future_part = message_row(
      "msg_user_future",
      json!({"role": "user"}),
      vec![
        part_row("prt_text", json!({"type": "text", "text": "visible"})),
        part_row("prt_future", json!({"type": "future-part", "answer": 42})),
      ],
    );
    let events = normalizer.normalize_message(user_with_future_part);
    assert!(matches!(&events[0], AgentEvent::Message(event)
      if matches!(event.role, Role::User) && event.text == "visible"));
    assert!(matches!(&events[1], AgentEvent::Unknown(event)
      if event.native_type.as_deref() == Some("part.future-part")
        && event.native.as_ref().and_then(|native| native.get("answer")) == Some(&json!(42))));

    let malformed_assistant = message_row(
      "msg_malformed_assistant",
      json!({
        "role": "assistant",
        "providerID": 42,
        "error": null
      }),
      vec![part_row(
        "prt_recoverable",
        json!({"type": "text", "text": "still visible"}),
      )],
    );
    let events = normalizer.normalize_message(malformed_assistant);
    assert!(matches!(&events[0], AgentEvent::Message(event)
      if matches!(event.role, Role::Assistant) && event.text == "still visible"));
    assert!(matches!(&events[1], AgentEvent::Unknown(event)
      if event.native_type.as_deref() == Some("message.role.assistant")
        && event.native.as_ref().and_then(|native| native.get("providerID")) == Some(&json!(42))));
    assert_eq!(events.len(), 2);
  }

  #[test]
  fn keeps_historical_system_messages_and_error_state_detail() {
    let mut normalizer = OpenCodeNormalizer::new("ses_1".to_string());
    let system = message_row(
      "msg_system",
      json!({"role": "system"}),
      vec![part_row("prt_system", json!({"type": "text", "text": "context"}))],
    );
    let events = normalizer.normalize_message(system);
    assert!(matches!(&events[0], AgentEvent::Message(event)
      if matches!(event.role, Role::System) && event.text == "context"));

    let tool = message_row(
      "msg_assistant",
      json!({"role": "assistant"}),
      vec![part_row(
        "prt_tool_fallback",
        json!({
          "type": "tool",
          "tool": "bash",
          "state": {
            "status": "error",
            "input": {"command": "cargo test"},
            "raw": "{\"command\":\"cargo test\"}",
            "error": "failed"
          }
        }),
      )],
    );
    let events = normalizer.normalize_message(tool);
    let AgentEvent::ToolCall(event) = &events[0] else {
      panic!("expected tool event");
    };
    assert_eq!(event.tool_call_id.as_deref(), Some("prt_tool_fallback"));
    assert_eq!(
      event.output.as_ref().and_then(|output| output.get("raw")),
      Some(&Value::String("{\"command\":\"cargo test\"}".to_string()))
    );
  }

  fn message_row(id: &str, data: Value, parts: Vec<OpenCodePartRow>) -> OpenCodeMessageRow {
    OpenCodeMessageRow {
      id: id.to_string(),
      time_created: Some(1),
      data: serde_json::from_value::<MessageData>(data).expect("message should decode"),
      parts,
    }
  }

  fn part_row(id: &str, data: Value) -> OpenCodePartRow {
    OpenCodePartRow {
      id: id.to_string(),
      time_created: Some(2),
      data: serde_json::from_value::<PartData>(data).expect("part should decode"),
    }
  }
}
