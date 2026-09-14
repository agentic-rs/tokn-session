use serde_json::Value;
use tokn_codex_protocol::{AgentMessageItem, ContentItem, InterAgentCommunicationItem};
use tokn_session_core::{AgentActivity, AgentCommunication, AgentEvent, Provider};

use super::{json_value, unknown_event};

pub(super) fn normalize_agent_message(
  session_id: Option<String>,
  item: AgentMessageItem,
  timestamp: Option<String>,
  trigger_turn: Option<bool>,
) -> Vec<AgentEvent> {
  let native = json_value(&item);
  let communication = readable_parts(
    &item.content,
    item.author.as_deref(),
    item.recipient.as_deref(),
    trigger_turn,
  );
  vec![
    CommunicationRecord {
      event_id: item.id,
      author: item.author,
      recipient: item.recipient,
      native_type: "response_item.agent_message",
      native,
    }
    .normalize(session_id, communication, timestamp),
  ]
}

pub(super) fn normalize_inter_agent_communication(
  session_id: Option<String>,
  item: InterAgentCommunicationItem,
  timestamp: Option<String>,
) -> Vec<AgentEvent> {
  let native = json_value(&item);
  let communication = (item.content.is_some() || item.encrypted_content.is_some())
    .then(|| {
      if item.encrypted_content.as_deref() == Some("") {
        return None;
      }
      let has_encrypted_content = item.encrypted_content.is_some();
      let text = item.content.as_deref().filter(|text| {
        !text.is_empty()
          && !(has_encrypted_content
            && is_routing_header(
              text,
              item.author.as_deref(),
              item.recipient.as_deref(),
              item.trigger_turn,
            ))
      });
      Some(AgentCommunication {
        text: text.map(str::to_owned),
        has_encrypted_content,
        trigger_turn: item.trigger_turn,
      })
    })
    .flatten();
  vec![
    CommunicationRecord {
      event_id: item.id,
      author: item.author,
      recipient: item.recipient,
      native_type: "inter_agent_communication",
      native,
    }
    .normalize(session_id, communication, timestamp),
  ]
}

fn readable_parts(
  content: &[ContentItem],
  author: Option<&str>,
  recipient: Option<&str>,
  trigger_turn: Option<bool>,
) -> Option<AgentCommunication> {
  if content.is_empty() {
    return None;
  }
  let mut has_encrypted_content = false;
  // Validate the whole message before projecting any prose. An unsupported
  // part must stay discoverable as unknown, never become a partial message.
  for part in content {
    match part.content_type.as_deref() {
      Some("input_text" | "output_text")
        if part.text.is_some()
          && part.encrypted_content.is_none()
          && part.image_url.is_none()
          && part.audio_url.is_none() => {}
      Some("encrypted_content")
        if part
          .encrypted_content
          .as_deref()
          .is_some_and(|content| !content.is_empty())
          && part.text.is_none()
          && part.image_url.is_none()
          && part.audio_url.is_none() =>
      {
        has_encrypted_content = true;
      }
      _ => return None,
    }
  }
  let text = content
    .iter()
    .enumerate()
    .filter_map(|(index, part)| {
      let text = part.text.as_deref()?;
      // Codex prepends this exact input-text envelope to encrypted payloads.
      // Preserve all real prose, including plaintext headers without encryption.
      if text.is_empty()
        || (index == 0
          && has_encrypted_content
          && part.content_type.as_deref() == Some("input_text")
          && is_routing_header(text, author, recipient, trigger_turn))
      {
        None
      } else {
        Some(text)
      }
    })
    .collect::<Vec<_>>()
    .join("\n");
  Some(AgentCommunication {
    text: (!text.is_empty()).then_some(text),
    has_encrypted_content,
    trigger_turn,
  })
}

fn is_routing_header(text: &str, author: Option<&str>, recipient: Option<&str>, trigger_turn: Option<bool>) -> bool {
  let (Some(author), Some(recipient)) = (author, recipient) else {
    return false;
  };
  [false, true].into_iter().any(|trigger| {
    trigger_turn.is_none_or(|flag| flag == trigger)
      && text
        == format!(
          "Message Type: {}\nTask name: {recipient}\nSender: {author}\nPayload:\n",
          if trigger { "NEW_TASK" } else { "MESSAGE" }
        )
  })
}

struct CommunicationRecord {
  event_id: Option<String>,
  author: Option<String>,
  recipient: Option<String>,
  native_type: &'static str,
  native: Value,
}

impl CommunicationRecord {
  fn normalize(
    self,
    session_id: Option<String>,
    communication: Option<AgentCommunication>,
    timestamp: Option<String>,
  ) -> AgentEvent {
    if communication.is_none()
      || self.author.as_deref().is_none_or(|author| author.is_empty())
      || self.recipient.as_deref().is_none_or(|recipient| recipient.is_empty())
    {
      return unknown_event(session_id, Some(self.native_type.into()), Some(self.native), timestamp);
    }
    AgentEvent::AgentActivity(AgentActivity {
      provider: Provider::Codex,
      session_id,
      event_id: self.event_id,
      actor_session_id: None,
      actor_agent_path: self.author,
      target_session_id: None,
      target_agent_path: self.recipient,
      kind: "messaged".into(),
      communication,
      occurred_at_ms: None,
      native: Some(self.native),
      timestamp,
    })
  }
}
