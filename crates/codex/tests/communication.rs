use serde_json::{Value, json};
use tokn_session_codex::normalize::CodexNormalizer;
use tokn_session_core::{AgentActivity, AgentCommunication, AgentEvent, SessionHistoryStatus};

fn line(normalizer: &mut CodexNormalizer, native: Value) -> Vec<AgentEvent> {
  normalizer.normalize(serde_json::from_value(native).unwrap())
}

fn normalizer(paginated: bool) -> CodexNormalizer {
  let mut normalizer = CodexNormalizer::new();
  line(
    &mut normalizer,
    json!({"type":"session_meta","payload":{"id":"recipient-session",
      "history_mode":if paginated { "paginated" } else { "legacy" }}}),
  );
  normalizer
}

fn metadata(trigger_turn: bool) -> Value {
  json!({"type":"inter_agent_communication_metadata","payload":{"trigger_turn":trigger_turn}})
}

fn message(content: Value) -> Value {
  json!({"type":"response_item","timestamp":"2026-09-14T00:00:00Z","payload":{
    "type":"agent_message","id":"amsg-1","author":"/root/worker","recipient":"/root",
    "content":content,"future_field":"retained"}})
}

fn plaintext(text: &str) -> Value {
  message(json!([{"type":"input_text","text":text}]))
}

fn activity(events: &[AgentEvent]) -> &AgentActivity {
  let [AgentEvent::AgentActivity(activity)] = events else {
    panic!("expected one communication activity, got {events:?}");
  };
  activity
}

fn communication(events: &[AgentEvent]) -> &AgentCommunication {
  activity(events).communication.as_ref().expect("typed communication")
}

#[test]
fn inbound_plaintext_is_activity_in_both_history_modes() {
  for paginated in [false, true] {
    let events = line(&mut normalizer(paginated), plaintext("**Review** complete."));
    let activity = activity(&events);
    assert_eq!(activity.session_id.as_deref(), Some("recipient-session"));
    assert_eq!(activity.event_id.as_deref(), Some("amsg-1"));
    assert_eq!(activity.actor_agent_path.as_deref(), Some("/root/worker"));
    assert_eq!(activity.target_agent_path.as_deref(), Some("/root"));
    assert_eq!(activity.timestamp.as_deref(), Some("2026-09-14T00:00:00Z"));
    assert_eq!(activity.native.as_ref().unwrap()["future_field"], "retained");
    let communication = communication(&events);
    assert_eq!(communication.text.as_deref(), Some("**Review** complete."));
    assert!(!communication.has_encrypted_content);
    assert_eq!(communication.trigger_turn, None);
  }
}

#[test]
fn adjacent_metadata_adds_delivery_flag_without_replacing_metadata_record() {
  for paginated in [false, true] {
    for trigger in [false, true] {
      let mut normalizer = normalizer(paginated);
      let marker = line(&mut normalizer, metadata(trigger));
      assert!(matches!(&marker[..], [AgentEvent::Metadata(event)]
        if event.native_type == "inter_agent_communication_metadata"));
      let events = line(&mut normalizer, plaintext("next task"));
      assert_eq!(communication(&events).trigger_turn, Some(trigger));
      let next = line(&mut normalizer, plaintext("a separate message"));
      assert_eq!(communication(&next).trigger_turn, None);
    }
  }
}

#[test]
fn intervening_and_malformed_records_break_metadata_correlation() {
  for paginated in [false, true] {
    for intervening in [
      json!({"type":"turn_context","payload":{"turn_id":"turn-2"}}),
      json!({"type":"future_record","payload":{}}),
      json!({"type":"session_meta","payload":{"id":"copied-parent"}}),
      json!({"type":"inter_agent_communication_metadata","payload":{"trigger_turn":"yes"}}),
      message(json!([{"type":"future_content","text":"unverified"}])),
    ] {
      let mut normalizer = normalizer(paginated);
      line(&mut normalizer, metadata(true));
      line(&mut normalizer, intervening);
      let events = line(&mut normalizer, plaintext("unrelated delivery"));
      assert_eq!(communication(&events).trigger_turn, None);
    }
  }
}

#[test]
fn consumed_child_boundary_attaches_to_first_owned_delivery() {
  for paginated in [false, true] {
    let mut normalizer = CodexNormalizer::new_historical();
    line(
      &mut normalizer,
      json!({"type":"session_meta","payload":{"id":"child",
        "history_mode":if paginated { "paginated" } else { "legacy" },
        "source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}}}),
    );
    assert!(line(&mut normalizer, metadata(false)).is_empty());
    assert!(line(&mut normalizer, plaintext("copied parent delivery")).is_empty());
    assert!(line(&mut normalizer, metadata(true)).is_empty());
    let events = line(&mut normalizer, plaintext("owned first task"));
    assert_eq!(communication(&events).trigger_turn, Some(true));
    assert_eq!(communication(&events).text.as_deref(), Some("owned first task"));
    assert_eq!(normalizer.history_status(), SessionHistoryStatus::FilteredSubagent);
    assert_eq!(activity(&events).session_id.as_deref(), Some("child"));
  }
}

#[test]
fn filtered_copied_session_meta_does_not_leak_a_pending_delivery_flag() {
  let mut normalizer = CodexNormalizer::new_historical();
  line(&mut normalizer, json!({"type":"session_meta","payload":{"id":"root"}}));
  line(&mut normalizer, metadata(true));
  assert!(
    line(
      &mut normalizer,
      json!({"type":"session_meta","payload":{"id":"copied"}})
    )
    .is_empty()
  );
  let events = line(&mut normalizer, plaintext("separate message"));
  assert_eq!(communication(&events).trigger_turn, None);
}

fn envelope(message_type: &str) -> String {
  format!("Message Type: {message_type}\nTask name: /root\nSender: /root/worker\nPayload:\n")
}

#[test]
fn encrypted_routing_envelope_does_not_masquerade_as_readable_body() {
  for paginated in [false, true] {
    for trigger in [None, Some(false), Some(true)] {
      let mut normalizer = normalizer(paginated);
      if let Some(trigger) = trigger {
        line(&mut normalizer, metadata(trigger));
      }
      let events = line(
        &mut normalizer,
        message(json!([
          {"type":"input_text","text":envelope(if trigger == Some(true) { "NEW_TASK" } else { "MESSAGE" })},
          {"type":"encrypted_content","encrypted_content":"opaque-secret"}
        ])),
      );
      let communication = communication(&events);
      assert!(communication.text.is_none());
      assert!(communication.has_encrypted_content);
      assert_eq!(communication.trigger_turn, trigger);
      assert!(!serde_json::to_string(communication).unwrap().contains("opaque-secret"));
      assert_eq!(
        activity(&events).native.as_ref().unwrap()["content"][1]["encrypted_content"],
        "opaque-secret"
      );
    }
  }
}

#[test]
fn mixed_readable_and_encrypted_parts_preserve_real_prose() {
  for paginated in [false, true] {
    let events = line(
      &mut normalizer(paginated),
      message(json!([
        {"type":"input_text","text":envelope("MESSAGE")},
        {"type":"encrypted_content","encrypted_content":"opaque"},
        {"type":"input_text","text":"Visible **context**"},
        {"type":"output_text","text":"Second paragraph"}
      ])),
    );
    assert_eq!(
      communication(&events).text.as_deref(),
      Some("Visible **context**\nSecond paragraph")
    );
    assert!(communication(&events).has_encrypted_content);
  }
}

#[test]
fn only_exact_encrypted_routing_header_is_suppressed() {
  let header = envelope("MESSAGE");
  let events = line(&mut normalizer(true), plaintext(&header));
  assert_eq!(communication(&events).text.as_deref(), Some(header.as_str()));
  for text in [
    format!("{header}actual plaintext"),
    header.replace("/root/worker", "/root/other"),
    header.replace("Task name: /root", "Task name: /root/other"),
    header.trim_end().to_owned(),
  ] {
    let events = line(
      &mut normalizer(true),
      message(json!([
        {"type":"input_text","text":text},
        {"type":"encrypted_content","encrypted_content":"opaque"}
      ])),
    );
    assert_eq!(communication(&events).text.as_deref(), Some(text.as_str()));
  }
}

#[test]
fn unsupported_or_malformed_parts_remain_unknown_without_partial_prose() {
  for paginated in [false, true] {
    for content in [
      json!([]),
      json!([{"type":"input_text"}]),
      json!([{"type":"input_text","text":42}]),
      json!([{"type":"encrypted_content"}]),
      json!([{"type":"encrypted_content","encrypted_content":""}]),
      json!([{"type":"encrypted_content","encrypted_content":"opaque","text":"misleading"}]),
      json!([{"type":"input_text","text":"misleading","encrypted_content":"opaque"}]),
      json!([{"type":"input_text","text":"known"},{"type":"input_image","image_url":"private"}]),
      json!([{"type":"input_text","text":"known"},{"type":"future_content","text":"unverified"}]),
    ] {
      let native = message(content);
      let events = line(&mut normalizer(paginated), native);
      assert!(matches!(&events[..], [AgentEvent::Unknown(event)]
        if event.native_type.as_deref() == Some("response_item.agent_message") && event.native.is_some()));
    }
  }
}

#[test]
fn legacy_delivery_uses_same_typed_communication_in_both_modes() {
  for paginated in [false, true] {
    for (content, encrypted_content) in [
      (Some("result"), None),
      (None, Some("opaque")),
      (Some("context"), Some("opaque")),
    ] {
      let events = line(
        &mut normalizer(paginated),
        json!({"type":"inter_agent_communication","payload":{
          "id":"legacy-1","author":"/root/worker","recipient":"/root",
          "content":content,"encrypted_content":encrypted_content,"trigger_turn":false}}),
      );
      assert_eq!(communication(&events).text.as_deref(), content);
      assert_eq!(
        communication(&events).has_encrypted_content,
        encrypted_content.is_some()
      );
      assert_eq!(communication(&events).trigger_turn, Some(false));
    }
  }
}

#[test]
fn malformed_legacy_deliveries_remain_unknown() {
  for payload in [
    json!({"author":"/root/worker","recipient":"/root"}),
    json!({"author":"/root/worker","recipient":"/root","content":42}),
    json!({"author":"/root/worker","recipient":"/root","encrypted_content":""}),
    json!({"author":"/root/worker","content":"missing recipient"}),
  ] {
    let events = line(
      &mut normalizer(true),
      json!({"type":"inter_agent_communication","payload":payload}),
    );
    assert!(matches!(&events[..], [AgentEvent::Unknown(_)]));
  }
}

#[test]
fn old_activity_wire_format_remains_compatible() {
  let native = json!({"provider":"codex","kind":"spawned","session_id":"old"});
  let activity: AgentActivity = serde_json::from_value(native).unwrap();
  assert!(activity.communication.is_none());
  assert!(serde_json::to_value(activity).unwrap().get("communication").is_none());
}
