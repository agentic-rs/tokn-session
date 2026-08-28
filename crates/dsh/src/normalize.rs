use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use tokn_dsh_protocol::{ContentBlock, DshSessionItem, DshSessionLine, SessionEvent, SessionHeader, StreamChunk};
use tokn_session_core::{
  AgentEvent, ErrorEvent, MessageDelivery, MessageEvent, Phase, Provider, ProviderChanged, ReasoningEvent, Role,
  SessionStarted, ToolCallEvent, UnknownEvent, tool_kind_for_optional_name, tool_summary_for_io,
};

/// Historical log view, not a reconstruction of the compacted model surface.
/// Assembled messages replace their redundant stream deltas for display, while
/// unfinished steps still expose their partial text and reasoning.
pub(crate) fn normalize(header: &SessionHeader, lines: Vec<DshSessionLine>) -> Vec<AgentEvent> {
  let mut state = Normalizer {
    session_id: header.id.clone(),
    assembled: HashSet::new(),
    calls: HashMap::new(),
    emitted_calls: HashSet::new(),
  };
  for line in &lines {
    match line.item() {
      DshSessionItem::Event(SessionEvent::AssistantMessage(event)) => {
        state.assembled.insert((event.data.turn, event.data.step));
      }
      DshSessionItem::Event(SessionEvent::ToolCall(event)) => {
        state.calls.insert(
          (event.data.turn, event.data.step, event.data.call_id.clone()),
          (event.data.name.clone(), arguments(&event.data.arguments)),
        );
      }
      _ => {}
    }
  }
  let mut events = vec![AgentEvent::SessionStarted(SessionStarted {
    provider: Provider::Dsh,
    session_id: header.id.clone(),
    cwd: header.cwd.clone(),
    timestamp: Some(header.created_at.to_string()),
  })];
  for line in lines {
    events.extend(state.line(line));
  }
  events
}

type CallKey = (u64, u64, String);

struct Normalizer {
  session_id: String,
  assembled: HashSet<(u64, u64)>,
  calls: HashMap<CallKey, (String, Value)>,
  emitted_calls: HashSet<CallKey>,
}

impl Normalizer {
  fn line(&mut self, line: DshSessionLine) -> Vec<AgentEvent> {
    let time = line
      .native()
      .get("time")
      .and_then(Value::as_i64)
      .map(|time| time.to_string());
    let (item, native) = line.into_parts();
    let DshSessionItem::Event(event) = item else {
      return vec![self.unknown(native, time)];
    };
    match event {
      SessionEvent::RequestHeader(event) => vec![AgentEvent::ProviderChanged(ProviderChanged {
        provider: Provider::Dsh,
        session_id: Some(self.session_id.clone()),
        native_id: Some(event.seq.to_string()),
        native_parent_id: None,
        model_provider: Some(event.data.header.config.provider),
        model_id: Some(event.data.header.config.model),
        thinking_level: event.data.header.config.reasoning_effort,
        timestamp: time,
      })],
      SessionEvent::UserMessage(event) => {
        self.message(&event.data.id, &event.data.role, event.data.content, None, native, time)
      }
      SessionEvent::AssistantMessage(event) => {
        let usage = event.data.usage.as_ref().map(|_| {
          json!({
            "type": "assistant/message.usage", "seq": event.seq,
            "turn": event.data.turn, "step": event.data.step,
            "message_id": event.data.message.id, "usage": native["data"]["usage"]
          })
        });
        let mut events = self.message(
          &event.data.message.id,
          &event.data.message.role,
          event.data.message.content,
          Some((event.data.turn, event.data.step)),
          native,
          time.clone(),
        );
        if let Some(usage) = usage {
          events.push(self.unknown(usage, time));
        }
        events
      }
      SessionEvent::ToolCall(event) => {
        let key = (event.data.turn, event.data.step, event.data.call_id.clone());
        if !self.emitted_calls.insert(key) {
          return vec![];
        }
        vec![self.tool(
          Some(event.data.call_id),
          Some(event.data.name),
          None,
          Some(arguments(&event.data.arguments)),
          None,
          None,
          Phase::Started,
          time,
        )]
      }
      SessionEvent::ToolResult(event) => {
        let mut events = Vec::new();
        for block in event.data.message.content {
          let ContentBlock::ToolResult(block) = block else {
            events.push(self.unknown(native.clone(), time.clone()));
            continue;
          };
          let call = self
            .calls
            .get(&(event.data.turn, event.data.step, block.tool_call_id.clone()));
          let output = json!({
            "content": native["data"]["message"]["content"],
            "meta": native["data"]["meta"],
            "error": native["data"]["error"]
          });
          events.push(self.tool(
            Some(block.tool_call_id),
            call.map(|(name, _)| name.clone()),
            Some(event.data.message.id.clone()),
            call.map(|(_, input)| input.clone()),
            Some(output),
            Some(block.is_error.unwrap_or(false) || event.data.error.is_some()),
            Phase::Finished,
            time.clone(),
          ));
        }
        if events.is_empty() {
          events.push(self.unknown(native, time));
        }
        events
      }
      SessionEvent::AssistantChunk(event) => {
        let step = (event.data.turn, event.data.step);
        match event.data.chunk {
          StreamChunk::Unknown(_) => vec![self.unknown(native, time)],
          StreamChunk::BlockEnd(ref chunk) if matches!(chunk.block, ContentBlock::Unknown(_)) => {
            vec![self.unknown(native, time)]
          }
          // Usage and failures have no dedicated IR yet; retain the whole
          // record rather than discarding provider facts.
          StreamChunk::Usage(_) => vec![self.unknown(native, time)],
          StreamChunk::Finish(chunk) if !matches!(chunk.reason.kind.as_str(), "stop" | "tool-calls" | "max-tokens") => {
            vec![self.unknown(native, time)]
          }
          _ if self.assembled.contains(&step) => vec![],
          StreamChunk::TextDelta(chunk) => vec![self.text(
            None,
            Role::Assistant,
            MessageDelivery::Unspecified,
            Phase::Delta,
            chunk.text,
            time,
          )],
          StreamChunk::ReasoningDelta(chunk) => vec![self.reasoning(None, Phase::Delta, chunk.text, time)],
          // Without an assembled message, raw tool fragments/block boundaries
          // remain visible and do not pretend to be completed tool calls.
          _ => vec![self.unknown(native, time)],
        }
      }
      SessionEvent::TurnEnd(event) if event.data.reason.kind == "error" => {
        let message = event
          .data
          .reason
          .extra
          .get("error")
          .and_then(|error| error.get("message"))
          .and_then(Value::as_str)
          .unwrap_or("DSH turn failed")
          .to_string();
        vec![
          AgentEvent::Error(ErrorEvent {
            provider: Provider::Dsh,
            session_id: Some(self.session_id.clone()),
            message,
            timestamp: time.clone(),
          }),
          self.unknown(native, time),
        ]
      }
      // Lifecycle, title, compaction/surface operations, plugin records and
      // future vocabulary remain inspectable in the chronological log view.
      _ => vec![self.unknown(native, time)],
    }
  }

  fn message(
    &mut self,
    id: &str,
    role: &str,
    blocks: Vec<ContentBlock>,
    step: Option<(u64, u64)>,
    native: Value,
    time: Option<String>,
  ) -> Vec<AgentEvent> {
    let role = match role {
      "user" => Role::User,
      "assistant" => Role::Assistant,
      "system" => Role::System,
      _ => Role::Unknown,
    };
    let has_calls = blocks.iter().any(|block| matches!(block, ContentBlock::ToolCall(_)));
    let delivery = if matches!(role, Role::Assistant) {
      if has_calls {
        MessageDelivery::Commentary
      } else {
        MessageDelivery::Final
      }
    } else {
      MessageDelivery::Unspecified
    };
    let mut events = Vec::new();
    let mut preserve_native = blocks.is_empty();
    for block in blocks {
      match block {
        ContentBlock::Text(block) => events.push(self.text(
          Some(id.into()),
          role,
          delivery,
          Phase::Finished,
          block.text,
          time.clone(),
        )),
        ContentBlock::Reasoning(block) => {
          events.push(self.reasoning(Some(id.into()), Phase::Finished, block.text, time.clone()))
        }
        ContentBlock::ToolCall(block) if step.is_some() => {
          let (turn, step) = step.unwrap();
          let key = (turn, step, block.id.clone());
          // The explicit tool/call is the execution boundary; assistant blocks
          // are only a fallback for imported logs without that record.
          if !self.calls.contains_key(&key) && self.emitted_calls.insert(key.clone()) {
            let input = arguments(&block.arguments);
            self.calls.insert(key, (block.name.clone(), input.clone()));
            events.push(self.tool(
              Some(block.id),
              Some(block.name),
              Some(id.into()),
              Some(input),
              None,
              None,
              Phase::Started,
              time.clone(),
            ));
          }
        }
        _ => preserve_native = true,
      }
    }
    // Nonstandard sources and surface replacements affect provenance; retain
    // them alongside readable text until the shared IR can express them.
    let message = if native["type"] == "user/message" {
      &native["data"]
    } else {
      &native["data"]["message"]
    };
    let source = message["source"]["kind"].as_str();
    if preserve_native
      || !matches!(source, Some("user" | "model"))
      || native.get("surfaceOp").is_some_and(Value::is_object)
    {
      events.push(self.unknown(native, time));
    }
    events
  }

  fn text(
    &self,
    id: Option<String>,
    role: Role,
    delivery: MessageDelivery,
    phase: Phase,
    text: String,
    time: Option<String>,
  ) -> AgentEvent {
    AgentEvent::Message(MessageEvent {
      provider: Provider::Dsh,
      session_id: Some(self.session_id.clone()),
      message_id: id,
      parent_id: None,
      role,
      delivery,
      phase,
      text,
      timestamp: time,
    })
  }

  fn reasoning(&self, id: Option<String>, phase: Phase, text: String, time: Option<String>) -> AgentEvent {
    AgentEvent::Reasoning(ReasoningEvent {
      provider: Provider::Dsh,
      session_id: Some(self.session_id.clone()),
      message_id: id,
      parent_id: None,
      phase,
      text: Some(text),
      summary: None,
      encrypted_content: None,
      signature: None,
      timestamp: time,
    })
  }

  #[allow(clippy::too_many_arguments)]
  fn tool(
    &self,
    id: Option<String>,
    name: Option<String>,
    message_id: Option<String>,
    input: Option<Value>,
    output: Option<Value>,
    is_error: Option<bool>,
    phase: Phase,
    time: Option<String>,
  ) -> AgentEvent {
    AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Dsh,
      session_id: Some(self.session_id.clone()),
      message_id,
      parent_id: None,
      tool_call_id: id,
      tool_kind: tool_kind_for_optional_name(name.as_deref()),
      summary: tool_summary_for_io(name.as_deref(), input.as_ref(), output.as_ref()),
      tool_name: name,
      phase,
      input,
      output,
      is_error,
      timestamp: time,
    })
  }

  fn unknown(&self, native: Value, time: Option<String>) -> AgentEvent {
    AgentEvent::Unknown(UnknownEvent {
      provider: Provider::Dsh,
      session_id: Some(self.session_id.clone()),
      native_type: native.get("type").and_then(Value::as_str).map(str::to_string),
      native: Some(native),
      timestamp: time,
    })
  }
}

fn arguments(raw: &str) -> Value {
  serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}
