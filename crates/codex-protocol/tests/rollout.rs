use serde_json::{Value, json};
use tokn_codex_protocol::{ResponseItem, RolloutItem, RolloutLine};

#[test]
fn decodes_current_rollout_control_records() {
  let world_state: RolloutLine = serde_json::from_value(json!({
    "timestamp": "2026-07-28T00:00:00Z",
    "ordinal": 7,
    "type": "world_state",
    "payload": {
      "full": false,
      "state": {
        "environments": {
          "subagents": ["reviewer"]
        }
      },
      "future_field": true
    }
  }))
  .expect("world state should decode");

  assert_eq!(world_state.timestamp(), Some("2026-07-28T00:00:00Z"));
  assert_eq!(world_state.ordinal(), Some(7));
  let RolloutItem::WorldState(item) = world_state.item() else {
    panic!("expected world state");
  };
  assert_eq!(item.full, Some(false));
  assert_eq!(item.extra.get("future_field"), Some(&Value::Bool(true)));

  let metadata: RolloutLine = serde_json::from_value(json!({
    "type": "inter_agent_communication_metadata",
    "payload": {
      "trigger_turn": true
    }
  }))
  .expect("communication metadata should decode");
  let RolloutItem::InterAgentCommunicationMetadata(item) = metadata.item() else {
    panic!("expected communication metadata");
  };
  assert_eq!(item.trigger_turn, Some(true));
}

#[test]
fn accepts_new_turn_context_values_without_schema_failure() {
  let line: RolloutLine = serde_json::from_value(json!({
    "type": "turn_context",
    "payload": {
      "turn_id": "turn-1",
      "cwd": "/tmp/project",
      "workspace_roots": ["/tmp/project"],
      "approval_policy": "on-request",
      "approvals_reviewer": "auto_review",
      "sandbox_policy": {"type": "workspace-write"},
      "model": "gpt-5.6-sol",
      "effort": "ultra",
      "collaboration_mode": {
        "mode": "default",
        "settings": {
          "future_setting": true
        }
      }
    }
  }))
  .expect("turn context should decode");

  let RolloutItem::TurnContext(item) = line.item() else {
    panic!("expected turn context");
  };
  assert_eq!(item.effort.as_deref(), Some("ultra"));
  assert_eq!(item.approvals_reviewer.as_deref(), Some("auto_review"));
  assert_eq!(item.workspace_roots, ["/tmp/project"]);
}

#[test]
fn decodes_agent_messages_without_erasing_identity() {
  let line: RolloutLine = serde_json::from_value(json!({
    "type": "response_item",
    "payload": {
      "type": "agent_message",
      "id": "amsg_1",
      "author": "/root",
      "recipient": "/root/reviewer",
      "content": [
        {
          "type": "input_text",
          "text": "Please review this."
        },
        {
          "type": "encrypted_content",
          "encrypted_content": "ciphertext"
        }
      ]
    }
  }))
  .expect("agent message should decode");

  let RolloutItem::ResponseItem(ResponseItem::AgentMessage(item)) = line.item() else {
    panic!("expected agent message");
  };
  assert_eq!(item.id.as_deref(), Some("amsg_1"));
  assert_eq!(item.author.as_deref(), Some("/root"));
  assert_eq!(item.recipient.as_deref(), Some("/root/reviewer"));
  assert_eq!(item.content[0].text.as_deref(), Some("Please review this."));
  assert_eq!(item.content[1].encrypted_content.as_deref(), Some("ciphertext"));
}

#[test]
fn accepts_string_and_structured_custom_tool_outputs() {
  let string_output = decode_response(json!({
    "type": "custom_tool_call_output",
    "call_id": "call-old",
    "output": "done"
  }));
  let ResponseItem::CustomToolCallOutput(item) = string_output else {
    panic!("expected custom tool output");
  };
  assert_eq!(item.output, json!("done"));

  let structured_output = decode_response(json!({
    "type": "custom_tool_call_output",
    "call_id": "call-new",
    "name": "exec",
    "output": [
      {
        "type": "input_text",
        "text": "Script completed"
      }
    ]
  }));
  let ResponseItem::CustomToolCallOutput(item) = structured_output else {
    panic!("expected structured custom tool output");
  };
  assert_eq!(item.name.as_deref(), Some("exec"));
  assert!(item.output.is_array());
}

#[test]
fn preserves_unknown_rollout_and_response_types() {
  let response_payload = json!({
    "type": "future_response",
    "id": "future-1",
    "data": {
      "answer": 42
    }
  });
  let response = decode_response(response_payload.clone());
  let ResponseItem::Unknown(item) = response else {
    panic!("expected unknown response");
  };
  assert_eq!(item.native_type.as_deref(), Some("future_response"));
  assert_eq!(item.payload, response_payload);
  assert!(item.parse_error.is_none());

  let rollout_payload = json!({
    "enabled": true
  });
  let line: RolloutLine = serde_json::from_value(json!({
    "type": "future_rollout",
    "payload": rollout_payload
  }))
  .expect("future rollout should decode");
  let RolloutItem::Unknown(item) = line.item() else {
    panic!("expected unknown rollout");
  };
  assert_eq!(item.native_type.as_deref(), Some("future_rollout"));
  assert_eq!(item.payload, rollout_payload);
}

#[test]
fn serializing_a_line_returns_the_unchanged_native_record() {
  let native = json!({
    "timestamp": "2026-07-28T00:00:00Z",
    "type": "future_rollout",
    "payload": {
      "nested": [1, 2, 3]
    },
    "top_level_extension": "preserved"
  });
  let line: RolloutLine = serde_json::from_value(native.clone()).expect("line should decode");
  assert_eq!(serde_json::to_value(line).expect("line should serialize"), native);
}

fn decode_response(payload: Value) -> ResponseItem {
  let line: RolloutLine = serde_json::from_value(json!({
    "type": "response_item",
    "payload": payload
  }))
  .expect("response item should decode");
  let RolloutItem::ResponseItem(item) = line.into_item() else {
    panic!("expected response item");
  };
  item
}
