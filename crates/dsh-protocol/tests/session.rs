use serde_json::{Value, json};
use tokn_dsh_protocol::{ContentBlock, DshSessionItem, DshSessionLine, SessionEvent, StreamChunk, SurfaceOp};

#[test]
fn decodes_header_and_core_conversation_events() {
  let header = decode(json!({
    "type": "session",
    "version": 0,
    "id": "session-1",
    "createdAt": 1_786_000_000_000_u64,
    "cwd": "/tmp/project",
    "delegationDepth": 0,
    "futureHeaderField": true
  }));
  let DshSessionItem::Session(header) = header.item() else {
    panic!("expected session header");
  };
  assert_eq!(header.id, "session-1");
  assert_eq!(header.version, 0);
  assert_eq!(header.extra.get("futureHeaderField"), Some(&Value::Bool(true)));

  let turn = decode(json!({
    "type": "turn/start",
    "seq": 0,
    "time": 1,
    "data": {"turn": 1}
  }));
  let DshSessionItem::Event(SessionEvent::TurnStart(turn)) = turn.item() else {
    panic!("expected turn start");
  };
  assert_eq!(turn.seq, 0);
  assert_eq!(turn.data.turn, 1);

  let user = decode(json!({
    "type": "user/message",
    "seq": 1,
    "time": 2,
    "surfaceOp": "append",
    "data": {
      "id": "message-1",
      "role": "user",
      "content": [{"type": "text", "text": "hello"}],
      "source": {"kind": "user"}
    }
  }));
  let DshSessionItem::Event(SessionEvent::UserMessage(user)) = user.item() else {
    panic!("expected user message");
  };
  assert!(matches!(user.surface_op, Some(SurfaceOp::Append)));
  assert!(matches!(&user.data.content[0], ContentBlock::Text(block) if block.text == "hello"));
}

#[test]
fn decodes_assistant_stream_messages_and_tools() {
  let chunk = decode(json!({
    "type": "assistant/chunk",
    "seq": 3,
    "time": 4,
    "data": {
      "turn": 1,
      "step": 1,
      "chunk": {
        "type": "tool-call-delta",
        "index": 0,
        "id": "call-1",
        "name": "read",
        "argumentsDelta": "{\"path\":"
      }
    }
  }));
  let DshSessionItem::Event(SessionEvent::AssistantChunk(chunk)) = chunk.item() else {
    panic!("expected assistant chunk");
  };
  assert!(matches!(&chunk.data.chunk, StreamChunk::ToolCallDelta(value) if value.id == "call-1"));

  let assistant = decode(json!({
    "type": "assistant/message",
    "seq": 4,
    "time": 5,
    "sourceEventSeqs": [3],
    "surfaceOp": {"op": "replace", "start": 1, "end": 1},
    "data": {
      "turn": 1,
      "step": 1,
      "message": {
        "id": "message-2",
        "role": "assistant",
        "content": [{"type": "tool-call", "id": "call-1", "name": "read", "arguments": "{}"}],
        "source": {"kind": "model", "provider": "deepseek", "model": "deepseek-chat"}
      },
      "usage": {"inputTokens": 10, "outputTokens": 2, "cacheReadTokens": 3}
    }
  }));
  let DshSessionItem::Event(SessionEvent::AssistantMessage(assistant)) = assistant.item() else {
    panic!("expected assistant message");
  };
  assert_eq!(assistant.data.usage.as_ref().map(|usage| usage.input_tokens), Some(10));
  assert!(matches!(assistant.surface_op, Some(SurfaceOp::Replace(_))));

  let result = decode(json!({
    "type": "tool/result",
    "seq": 5,
    "time": 6,
    "surfaceOp": "append",
    "data": {
      "turn": 1,
      "step": 1,
      "message": {
        "id": "message-3",
        "role": "user",
        "content": [{
          "type": "tool-result",
          "toolCallId": "call-1",
          "content": [{"type": "text", "text": "contents"}],
          "isError": false
        }],
        "source": {"kind": "tool", "callId": "call-1"}
      },
      "meta": {"presentation": "card"}
    }
  }));
  assert!(matches!(
    result.item(),
    DshSessionItem::Event(SessionEvent::ToolResult(_))
  ));
}

#[test]
fn decodes_request_state_and_packed_chunk_rows() {
  let header = decode(json!({
    "type": "request/header",
    "seq": 2,
    "time": 3,
    "data": {
      "reason": "initial",
      "header": {
        "config": {
          "provider": "deepseek",
          "model": "deepseek-chat",
          "reasoningEffort": "high",
          "maxTokens": 4096
        },
        "system": "Be useful",
        "tools": [{"name": "read", "description": "Read a file", "parameters": {"type": "object"}}]
      }
    }
  }));
  let DshSessionItem::Event(SessionEvent::RequestHeader(header)) = header.item() else {
    panic!("expected request header");
  };
  assert_eq!(header.data.header.config.reasoning_effort.as_deref(), Some("high"));

  let text = decode(json!({
    "type": "text-chunks",
    "seq0": 10,
    "time0": 20,
    "data": {"turn": 1, "step": 1, "index": 0, "dt": [1, -1], "texts": ["a", "b", "c"]}
  }));
  let DshSessionItem::TextChunks(text) = text.item() else {
    panic!("expected text chunks row");
  };
  assert_eq!(text.data.texts, ["a", "b", "c"]);

  let tool = decode(json!({
    "type": "tool-call-chunks",
    "seq0": 20,
    "time0": 30,
    "data": {
      "turn": 1,
      "step": 1,
      "index": 1,
      "id": "call-1",
      "name": "read",
      "dt": [1, 1],
      "args": ["{", "\"path\":", "\"README.md\"}"]
    }
  }));
  assert!(matches!(tool.item(), DshSessionItem::ToolCallChunks(_)));
}

#[test]
fn preserves_plugin_events_and_unknown_nested_variants() {
  let plugin = json!({
    "type": "plugin/example",
    "seq": 8,
    "time": 9,
    "ignorable": true,
    "data": {"answer": 42}
  });
  let line = decode(plugin.clone());
  let DshSessionItem::Unknown(item) = line.item() else {
    panic!("expected unknown plugin event");
  };
  assert_eq!(item.native_type.as_deref(), Some("plugin/example"));
  assert_eq!(item.native.get("ignorable"), Some(&Value::Bool(true)));
  assert!(item.parse_error.is_none());
  assert_eq!(serde_json::to_value(line).expect("line should serialize"), plugin);

  let line = decode(json!({
    "type": "assistant/chunk",
    "seq": 9,
    "time": 10,
    "data": {
      "turn": 1,
      "step": 1,
      "chunk": {"type": "audio-delta", "audio": "opaque"}
    }
  }));
  let DshSessionItem::Event(SessionEvent::AssistantChunk(event)) = line.item() else {
    panic!("expected assistant chunk event");
  };
  let StreamChunk::Unknown(chunk) = &event.data.chunk else {
    panic!("expected unknown stream chunk");
  };
  assert_eq!(chunk.native_type.as_deref(), Some("audio-delta"));
}

#[test]
fn malformed_known_records_fall_back_without_data_loss() {
  let native = json!({
    "type": "turn/start",
    "seq": "not-a-number",
    "time": 1,
    "data": {"turn": 1}
  });
  let line = decode(native.clone());
  let DshSessionItem::Unknown(item) = line.item() else {
    panic!("expected tolerant fallback");
  };
  assert_eq!(item.native_type.as_deref(), Some("turn/start"));
  assert!(item.parse_error.is_some());
  assert_eq!(item.native, native);
}

fn decode(native: Value) -> DshSessionLine {
  serde_json::from_value(native).expect("record should decode")
}
