use serde_json::{Value, json};
use tokn_opencode_protocol::run::{RunEvent, RunLine};

#[test]
fn decodes_every_current_run_event_and_preserves_lines() {
  let lines: Vec<&str> = include_str!("fixtures/run.jsonl").lines().collect();
  let expected = ["step_start", "reasoning", "text", "tool_use", "step_finish", "error"];

  for (line, expected_type) in lines.iter().zip(expected) {
    let native: Value = serde_json::from_str(line).expect("native run line should parse");
    let decoded: RunLine = serde_json::from_str(line).expect("run line should decode");
    assert_eq!(decoded.event().native_type(), Some(expected_type));
    assert_eq!(decoded.session_id(), Some("ses_example"));
    assert!(decoded.timestamp().is_some());
    assert_eq!(
      serde_json::to_value(&decoded).expect("run line should serialize"),
      native
    );
  }

  let text: RunLine = serde_json::from_str(lines[2]).expect("text line should decode");
  let RunEvent::Text(part) = text.event() else {
    panic!("expected text event");
  };
  assert_eq!(part.text, "done");
  assert_eq!(part.identity.id.as_deref(), Some("prt_text"));
  assert_eq!(part.identity.message_id.as_deref(), Some("msg_example"));
}

#[test]
fn preserves_unknown_run_envelopes() {
  let native = json!({
    "type": "future_event",
    "timestamp": 1710000000010_i64,
    "sessionID": "ses_example",
    "payload": {"answer": 42}
  });
  let line: RunLine = serde_json::from_value(native.clone()).expect("unknown run line should decode");
  let RunEvent::Unknown(item) = line.event() else {
    panic!("expected unknown run event");
  };
  assert_eq!(item.native_type.as_deref(), Some("future_event"));
  assert!(item.parse_error.is_none());
  assert_eq!(item.native, native);
  assert_eq!(serde_json::to_value(line).expect("line should serialize"), native);
}

#[test]
fn malformed_known_run_envelopes_fall_back_without_data_loss() {
  let cases = [
    json!({
      "type": "text",
      "timestamp": 1,
      "sessionID": "ses_example"
    }),
    json!({
      "type": "text",
      "timestamp": 2,
      "sessionID": "ses_example",
      "part": {
        "type": "reasoning",
        "text": "wrong embedded tag"
      }
    }),
    json!({
      "type": "text",
      "timestamp": 3,
      "sessionID": "ses_example",
      "part": {
        "type": "text",
        "text": 42
      }
    }),
    json!({
      "type": "error",
      "timestamp": 4,
      "sessionID": "ses_example"
    }),
  ];

  for native in cases {
    let line: RunLine =
      serde_json::from_value(native.clone()).expect("malformed known run line should remain decodable");
    let RunEvent::Unknown(item) = line.event() else {
      panic!("expected malformed run fallback");
    };
    assert!(item.parse_error.is_some());
    assert_eq!(item.native, native);
    assert_eq!(serde_json::to_value(line).expect("line should serialize"), native);
  }
}

#[test]
fn non_object_run_values_remain_unknown_and_lossless() {
  for native in [Value::Null, json!(["future"]), json!(42)] {
    let line: RunLine = serde_json::from_value(native.clone()).expect("arbitrary JSON should decode");
    let RunEvent::Unknown(item) = line.event() else {
      panic!("expected unknown run event");
    };
    assert!(item.native_type.is_none());
    assert!(item.parse_error.is_none());
    assert_eq!(serde_json::to_value(line).expect("line should serialize"), native);
  }
}
