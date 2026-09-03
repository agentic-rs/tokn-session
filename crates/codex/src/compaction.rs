use tokn_codex_protocol::{RolloutItem, RolloutLine};
use tokn_session_core::{AgentEvent, CompactionEvent, CompactionState, Provider, UnknownEvent};

#[derive(Default)]
pub(crate) struct Compactions {
  sequence: u64,
  /// A checkpoint is followed by its completion notice, possibly with token
  /// accounting in between. Any other source record breaks this correlation.
  pending: Option<String>,
}

impl Compactions {
  pub fn normalize(
    &mut self,
    line: &RolloutLine,
    session_id: Option<String>,
    canonical: bool,
  ) -> Option<Vec<AgentEvent>> {
    let raw = line.native();
    let payload = &raw["payload"];
    let mut event = CompactionEvent::new(Provider::Codex, session_id, CompactionState::Completed);
    event.timestamp = line.timestamp().map(str::to_owned);
    if let Some(ordinal) = line.ordinal() {
      event.source_refs.push(ordinal.to_string());
    }
    match line.item() {
      RolloutItem::Compacted(item) if item.message.is_some() => {
        self.sequence += 1;
        let id = item
          .window_id
          .clone()
          .unwrap_or_else(|| format!("checkpoint:{}", self.sequence));
        self.pending = Some(id.clone());
        event.compaction_id = Some(id);
        event.summary = item.message.clone().filter(|text| !text.trim().is_empty());
        event.summary_opaque = item.replacement_history.as_ref().is_some_and(|items| {
          items.iter().any(|item| match item {
            tokn_codex_protocol::ResponseItem::Compaction(item)
            | tokn_codex_protocol::ResponseItem::ContextCompaction(item) => item.encrypted_content.is_some(),
            _ => false,
          })
        });
        event.context.window_id = item.window_id.clone();
        event.context.previous_window_id = item.previous_window_id.clone();
        event.context.window_number = item.window_number;
      }
      RolloutItem::EventMessage(item) if item.event_type.as_deref() == Some("context_compacted") => {
        event.compaction_id = self.pending.take();
      }
      RolloutItem::EventMessage(item)
        if canonical
          && item.event_type.as_deref() == Some("item_completed")
          && payload["item"]["type"] == "ContextCompaction" =>
      {
        let pending = self.pending.take();
        for field in ["thread_id", "turn_id"] {
          if payload[field].as_str().is_none_or(|id| id.is_empty()) {
            return None;
          }
        }
        let id = payload["item"]["id"].as_str().filter(|id| !id.is_empty())?;
        event.source_refs.push(id.to_owned());
        event.compaction_id = pending.or_else(|| Some(id.to_owned()));
        event.turn_id = payload["turn_id"].as_str().map(str::to_owned);
        event.timestamp = event
          .timestamp
          .or_else(|| payload["completed_at_ms"].as_u64().map(|time| time.to_string()));
      }
      RolloutItem::ResponseItem(item)
        if matches!(
          item,
          tokn_codex_protocol::ResponseItem::Compaction(_) | tokn_codex_protocol::ResponseItem::ContextCompaction(_)
        ) =>
      {
        self.pending = None;
        if matches!(item, tokn_codex_protocol::ResponseItem::Compaction(control) if control.encrypted_content.is_none())
        {
          return Some(vec![AgentEvent::Unknown(UnknownEvent {
            provider: Provider::Codex,
            session_id: event.session_id,
            native_type: Some("response_item.compaction".into()),
            native: Some(raw.clone()),
            timestamp: event.timestamp,
          })]);
        }
        event.compaction_id = payload["id"].as_str().map(str::to_owned);
        event.summary_opaque = payload["encrypted_content"].is_string();
      }
      RolloutItem::EventMessage(item) if item.event_type.as_deref() == Some("token_count") => return None,
      _ => {
        self.pending = None;
        return None;
      }
    }
    Some(vec![AgentEvent::Compaction(event)])
  }
}
