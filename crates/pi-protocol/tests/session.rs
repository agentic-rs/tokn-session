use serde_json::{Value, json};
use tokn_pi_protocol::{ContentBlock, Message, PiSessionItem, PiSessionLine, UserContent};

#[test]
fn decodes_current_session_and_message_records() {
  let header: PiSessionLine = serde_json::from_value(json!({
    "type": "session",
    "version": 3,
    "id": "session-1",
    "timestamp": "2026-07-29T00:00:00Z",
    "cwd": "/tmp/project",
    "future": true
  }))
  .expect("header should decode");

  let PiSessionItem::Session(header) = header.item() else {
    panic!("expected session header");
  };
  assert_eq!(header.id.as_deref(), Some("session-1"));
  assert_eq!(header.version, Some(3));
  assert_eq!(header.extra.get("future"), Some(&Value::Bool(true)));

  let message = decode_message(json!({
    "role": "assistant",
    "content": [
      {"type": "thinking", "thinking": "checking", "thinkingSignature": "sig"},
      {"type": "toolCall", "id": "call-1", "name": "read", "arguments": {"path": "README.md"}}
    ],
    "provider": "openai",
    "model": "gpt-5",
    "timestamp": 1785254400000_u64
  }));
  let Message::Assistant(message) = message else {
    panic!("expected assistant message");
  };
  assert!(matches!(&message.content[0], ContentBlock::Thinking(item) if item.thinking.as_deref() == Some("checking")));
  assert!(matches!(&message.content[1], ContentBlock::ToolCall(item) if item.name.as_deref() == Some("read")));
}

#[test]
fn accepts_text_and_block_user_content() {
  let text = decode_message(json!({
    "role": "user",
    "content": "hello",
    "timestamp": 1
  }));
  assert!(
    matches!(text, Message::User(message) if matches!(&message.content, UserContent::Text(text) if text == "hello"))
  );

  let blocks = decode_message(json!({
    "role": "user",
    "content": [{"type": "text", "text": "hello"}],
    "timestamp": 1
  }));
  assert!(
    matches!(blocks, Message::User(message) if matches!(&message.content, UserContent::Blocks(blocks) if matches!(&blocks[0], ContentBlock::Text(item) if item.text.as_deref() == Some("hello"))))
  );
}

#[test]
fn preserves_unknown_message_roles_and_content_blocks() {
  let unknown_role = decode_message(json!({
    "role": "bashExecution",
    "command": "pwd",
    "output": "/tmp/project",
    "exitCode": 0
  }));
  let Message::Unknown(item) = unknown_role else {
    panic!("expected unknown message role");
  };
  assert_eq!(item.native_type.as_deref(), Some("bashExecution"));
  assert_eq!(item.native.get("command"), Some(&json!("pwd")));

  let message = decode_message(json!({
    "role": "assistant",
    "content": [{"type": "future_block", "answer": 42}]
  }));
  let Message::Assistant(message) = message else {
    panic!("expected assistant message");
  };
  let ContentBlock::Unknown(item) = &message.content[0] else {
    panic!("expected unknown content block");
  };
  assert_eq!(item.native_type.as_deref(), Some("future_block"));
  assert_eq!(item.native.get("answer"), Some(&json!(42)));
}

#[test]
fn decodes_current_control_and_extension_records() {
  let cases = [
    (
      json!({"type": "compaction", "id": "c1", "summary": "summary", "tokensBefore": 100}),
      "compaction",
    ),
    (
      json!({"type": "branch_summary", "id": "b1", "fromId": "m1", "summary": "summary"}),
      "branch_summary",
    ),
    (
      json!({"type": "custom", "id": "x1", "customType": "extension", "data": {"enabled": true}}),
      "custom",
    ),
    (
      json!({"type": "custom_message", "id": "x2", "customType": "extension", "content": "context", "display": true}),
      "custom_message",
    ),
    (
      json!({"type": "label", "id": "l1", "targetId": "m1", "label": "checkpoint"}),
      "label",
    ),
    (
      json!({"type": "session_info", "id": "i1", "name": "Research"}),
      "session_info",
    ),
    (json!({"type": "leaf", "id": "f1", "targetId": "m1"}), "leaf"),
    (
      json!({"type": "active_tools_change", "id": "a1", "activeToolNames": ["read"]}),
      "active_tools_change",
    ),
  ];

  for (native, expected_type) in cases {
    let line: PiSessionLine = serde_json::from_value(native).expect("record should decode");
    assert_eq!(line.item().native_type(), Some(expected_type));
  }
}

#[test]
fn malformed_known_records_fall_back_without_losing_native_json() {
  let line: PiSessionLine = serde_json::from_value(json!({
    "type": "model_change",
    "id": 42,
    "provider": "openai",
    "modelId": "gpt-5"
  }))
  .expect("line should decode");

  let PiSessionItem::Unknown(item) = line.item() else {
    panic!("expected tolerant fallback");
  };
  assert_eq!(item.native_type.as_deref(), Some("model_change"));
  assert_eq!(item.native.get("id"), Some(&json!(42)));
  assert!(item.parse_error.is_some());
}

#[test]
fn preserves_future_top_level_records_and_exact_line_serialization() {
  let native = json!({
    "type": "future_entry",
    "id": "future-1",
    "payload": {"answer": 42},
    "top_level_extension": true
  });
  let line: PiSessionLine = serde_json::from_value(native.clone()).expect("line should decode");
  let PiSessionItem::Unknown(item) = line.item() else {
    panic!("expected unknown entry");
  };
  assert_eq!(item.native_type.as_deref(), Some("future_entry"));
  assert_eq!(item.native.get("payload"), Some(&json!({"answer": 42})));
  assert_eq!(serde_json::to_value(line).expect("line should serialize"), native);
}

fn decode_message(message: Value) -> Message {
  let line: PiSessionLine = serde_json::from_value(json!({
    "type": "message",
    "id": "message-1",
    "message": message
  }))
  .expect("message line should decode");
  let PiSessionItem::Message(item) = line.into_item() else {
    panic!("expected message item");
  };
  item.message.expect("message should be present")
}
