use serde_json::{Value, json};
use tokn_opencode_protocol::v1::{
  MessageData, MessageItem, PartData, PartItem, SessionModel, ToolState, ToolStateItem,
};

#[test]
fn decodes_current_v1_messages_and_preserves_native_json() {
  let native: Vec<Value> =
    serde_json::from_str(include_str!("fixtures/v1-messages.json")).expect("message fixture should parse");

  let user: MessageData = serde_json::from_value(native[0].clone()).expect("user message should decode");
  assert_eq!(user.native_role(), Some("user"));
  let MessageItem::User(message) = user.item() else {
    panic!("expected user message");
  };
  let model = message.model.as_ref().expect("user model should decode");
  assert_eq!(model.provider_id.as_deref(), Some("openai"));
  assert_eq!(model.model_id.as_deref(), Some("gpt-5"));
  assert_eq!(message.extra.get("future_field"), Some(&Value::Bool(true)));
  assert_eq!(serde_json::to_value(&user).expect("user should serialize"), native[0]);

  let assistant: MessageData = serde_json::from_value(native[1].clone()).expect("assistant message should decode");
  let MessageItem::Assistant(message) = assistant.item() else {
    panic!("expected assistant message");
  };
  assert_eq!(message.parent_id.as_deref(), Some("msg_parent"));
  assert_eq!(message.provider_id.as_deref(), Some("openai"));
  assert_eq!(message.model_id.as_deref(), Some("gpt-5"));
  assert_eq!(
    serde_json::to_value(&assistant).expect("assistant should serialize"),
    native[1]
  );
}

#[test]
fn decodes_session_model_column() {
  let model: SessionModel = serde_json::from_value(json!({
    "id": "gpt-5",
    "providerID": "openai",
    "variant": "high",
    "future": true
  }))
  .expect("session model should decode");

  assert_eq!(model.id.as_deref(), Some("gpt-5"));
  assert_eq!(model.provider_id.as_deref(), Some("openai"));
  assert_eq!(model.variant.as_deref(), Some("high"));
  assert_eq!(model.extra.get("future"), Some(&Value::Bool(true)));
}

#[test]
fn decodes_every_v1_part_family_and_optional_hydrated_identity() {
  let native: Vec<Value> =
    serde_json::from_str(include_str!("fixtures/v1-parts.json")).expect("part fixture should parse");
  let expected = [
    "snapshot",
    "patch",
    "text",
    "reasoning",
    "file",
    "agent",
    "compaction",
    "subtask",
    "retry",
    "step-start",
    "step-finish",
    "tool",
  ];

  for (native, expected_type) in native.iter().zip(expected) {
    let part: PartData = serde_json::from_value(native.clone()).expect("part should decode");
    assert_eq!(part.native_type(), Some(expected_type));
    assert_eq!(serde_json::to_value(&part).expect("part should serialize"), *native);
  }

  let hydrated: PartData = serde_json::from_value(native[2].clone()).expect("hydrated text should decode");
  assert_eq!(hydrated.id(), Some("prt_text"));
  assert_eq!(hydrated.session_id(), Some("ses_example"));
  assert_eq!(hydrated.message_id(), Some("msg_example"));
  let PartItem::Text(text) = hydrated.item() else {
    panic!("expected text part");
  };
  assert_eq!(text.text, "hello");
  assert_eq!(text.extra.get("future_field"), Some(&json!("preserved")));
}

#[test]
fn decodes_every_tool_state_and_preserves_extensions() {
  let native: Vec<Value> =
    serde_json::from_str(include_str!("fixtures/v1-tool-states.json")).expect("tool-state fixture should parse");
  let expected = ["pending", "running", "completed", "error"];

  for (native, expected_status) in native.iter().zip(expected) {
    let state: ToolState = serde_json::from_value(native.clone()).expect("tool state should decode");
    assert_eq!(state.native_status(), Some(expected_status));
    assert_eq!(serde_json::to_value(&state).expect("state should serialize"), *native);
  }

  let error: ToolState = serde_json::from_value(native[3].clone()).expect("error state should decode");
  let ToolStateItem::Error(error) = error.item() else {
    panic!("expected error state");
  };
  assert_eq!(error.raw.as_deref(), Some("{\"command\":\"cargo test\"}"));
  assert_eq!(error.error.as_deref(), Some("failed"));
}

#[test]
fn preserves_unknown_roles_parts_and_states() {
  let unknown_message_native = json!({
    "role": "future-role",
    "payload": {"answer": 42}
  });
  let message: MessageData =
    serde_json::from_value(unknown_message_native.clone()).expect("unknown message should decode");
  let MessageItem::Unknown(item) = message.item() else {
    panic!("expected unknown message");
  };
  assert_eq!(item.native_type.as_deref(), Some("future-role"));
  assert!(item.parse_error.is_none());
  assert_eq!(item.native, unknown_message_native);

  let unknown_part_native = json!({
    "type": "future-part",
    "answer": 42
  });
  let part: PartData = serde_json::from_value(unknown_part_native.clone()).expect("unknown part should decode");
  let PartItem::Unknown(item) = part.item() else {
    panic!("expected unknown part");
  };
  assert_eq!(item.native_type.as_deref(), Some("future-part"));
  assert!(item.parse_error.is_none());
  assert_eq!(item.native, unknown_part_native);

  let unknown_state_native = json!({
    "status": "paused",
    "progress": 0.5
  });
  let state: ToolState = serde_json::from_value(unknown_state_native.clone()).expect("unknown state should decode");
  let ToolStateItem::Unknown(item) = state.item() else {
    panic!("expected unknown state");
  };
  assert_eq!(item.native_type.as_deref(), Some("paused"));
  assert!(item.parse_error.is_none());
  assert_eq!(item.native, unknown_state_native);
}

#[test]
fn malformed_known_values_fall_back_at_the_narrowest_boundary() {
  let malformed_message_native = json!({
    "role": "assistant",
    "providerID": 42
  });
  let message: MessageData =
    serde_json::from_value(malformed_message_native.clone()).expect("malformed message should remain decodable");
  let MessageItem::Unknown(item) = message.item() else {
    panic!("expected malformed message fallback");
  };
  assert_eq!(item.native_type.as_deref(), Some("assistant"));
  assert!(item.parse_error.is_some());
  assert_eq!(item.native, malformed_message_native);

  let malformed_part_native = json!({
    "type": "text",
    "text": 42
  });
  let part: PartData =
    serde_json::from_value(malformed_part_native.clone()).expect("malformed part should remain decodable");
  let PartItem::Unknown(item) = part.item() else {
    panic!("expected malformed part fallback");
  };
  assert_eq!(item.native_type.as_deref(), Some("text"));
  assert!(item.parse_error.is_some());
  assert_eq!(item.native, malformed_part_native);

  let malformed_state_native = json!({
    "type": "tool",
    "tool": "bash",
    "state": {
      "status": "error",
      "error": 42
    }
  });
  let part: PartData =
    serde_json::from_value(malformed_state_native.clone()).expect("malformed nested state should decode");
  let PartItem::Tool(tool) = part.item() else {
    panic!("tool part should remain typed");
  };
  let ToolStateItem::Unknown(item) = tool.state.item() else {
    panic!("expected malformed state fallback");
  };
  assert_eq!(item.native_type.as_deref(), Some("error"));
  assert!(item.parse_error.is_some());
  assert_eq!(
    serde_json::to_value(&part).expect("part should serialize"),
    malformed_state_native
  );
}

#[test]
fn non_object_payloads_remain_structurally_lossless_unknowns() {
  for native in [Value::Null, json!(["future"]), json!(42)] {
    let part: PartData = serde_json::from_value(native.clone()).expect("arbitrary JSON should decode");
    let PartItem::Unknown(item) = part.item() else {
      panic!("expected unknown part");
    };
    assert!(item.native_type.is_none());
    assert!(item.parse_error.is_none());
    assert_eq!(serde_json::to_value(part).expect("part should serialize"), native);
  }
}
