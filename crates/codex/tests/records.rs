use serde_json::{Value, json};
use tokn_session_codex::normalize::CodexNormalizer;
use tokn_session_core::{AgentEvent, MetadataKind, UsageKind};

fn line(normalizer: &mut CodexNormalizer, native: Value) -> Vec<AgentEvent> {
  normalizer.normalize(serde_json::from_value(native).unwrap())
}

fn normalizer() -> CodexNormalizer {
  let mut normalizer = CodexNormalizer::new();
  line(
    &mut normalizer,
    json!({"type":"session_meta","payload":{"id":"codex-1"}}),
  );
  normalizer
}

fn token_count(total: u64) -> Value {
  let counters = json!({"input_tokens":total-5,"output_tokens":5,"cached_input_tokens":10,
    "cache_write_input_tokens":2,"reasoning_output_tokens":2,"total_tokens":total});
  json!({"type":"event_msg","ordinal":42,"timestamp":"2026-08-28T00:00:00Z","payload":{
    "type":"token_count","info":{"total_token_usage":counters,"last_token_usage":counters,
      "model_context_window":100000,"future_field":true},"rate_limits":null
  }})
}

#[test]
fn token_count_is_a_replaceable_snapshot_without_double_counting_cache() {
  let mut normalizer = normalizer();
  let record = token_count(35);
  let events = line(&mut normalizer, record.clone());
  let [AgentEvent::Usage(event)] = &events[..] else {
    panic!("expected snapshot")
  };
  assert_eq!(event.kind, UsageKind::SessionSnapshot);
  assert_eq!(event.session_id.as_deref(), Some("codex-1"));
  assert_eq!(event.record_id.as_deref(), Some("42"));
  assert!(event.turn_id.is_none());
  assert!(event.message_id.is_none());
  assert_eq!(event.input_tokens, 30);
  assert_eq!(event.output_tokens, 5);
  assert_eq!(event.cache_read_tokens, Some(10));
  assert_eq!(event.cache_write_tokens, Some(2));
  assert_eq!(event.total_tokens, Some(35));
  assert_eq!(event.native, record["payload"]["info"]);
  assert!(line(&mut normalizer, record).is_empty());
  for total in [40, 20, 5] {
    let events = line(&mut normalizer, token_count(total));
    assert!(matches!(&events[..], [AgentEvent::Usage(event)] if event.total_tokens == Some(total)));
  }
}

#[test]
fn rate_limit_only_changes_do_not_repeat_usage() {
  let mut normalizer = normalizer();
  let mut record = token_count(35);
  line(&mut normalizer, record.clone());
  record["payload"]["rate_limits"] = json!({"primary":{"used_percent":25.0,"window_minutes":300},"plan_type":"pro"});
  let events = line(&mut normalizer, record.clone());
  assert!(matches!(&events[..], [AgentEvent::Metadata(event)] if matches!(event.kind, MetadataKind::Diagnostic)));
  assert!(line(&mut normalizer, record.clone()).is_empty());
  record["payload"]["rate_limits"] = Value::Null;
  assert!(matches!(&line(&mut normalizer, record)[..], [AgentEvent::Metadata(_)]));
}

#[test]
fn unavailable_usage_does_not_fabricate_zero_and_resets_duplicate_detection() {
  let mut normalizer = normalizer();
  line(&mut normalizer, token_count(35));
  let events = line(
    &mut normalizer,
    json!({"type":"event_msg","payload":{"type":"token_count","info":null}}),
  );
  assert!(matches!(&events[..], [AgentEvent::Metadata(event)] if event.summary == "usage unavailable"));
  assert!(matches!(
    &line(&mut normalizer, token_count(35))[..],
    [AgentEvent::Usage(_)]
  ));
}

#[test]
fn total_only_context_estimates_remain_visible() {
  let mut record = token_count(35);
  for field in ["total_token_usage", "last_token_usage"] {
    record["payload"]["info"][field] = json!({"input_tokens":0,"output_tokens":0,"cached_input_tokens":0,
      "reasoning_output_tokens":0,"total_tokens":100000});
  }
  let events = line(&mut normalizer(), record);
  assert!(
    matches!(&events[..], [AgentEvent::Usage(event)] if event.total_tokens == Some(100000) && event.input_tokens == 0)
  );
}

#[test]
fn malformed_usage_and_rate_limits_stay_unknown() {
  for (pointer, value) in [
    ("/payload/info/total_token_usage/input_tokens", json!(-1)),
    ("/payload/info/last_token_usage/output_tokens", json!("bad")),
    ("/payload/info/total_token_usage/total_tokens", Value::Null),
    ("/payload/rate_limits", json!({"primary":{}})),
    ("/payload/rate_limits", json!({"individual_limit":{}})),
    ("/payload/rate_limits", json!([])),
  ] {
    let mut record = token_count(35);
    *record.pointer_mut(pointer).unwrap() = value;
    let events = line(&mut normalizer(), record.clone());
    assert!(matches!(&events[..], [AgentEvent::Unknown(event)] if event.native.as_ref() == Some(&record)));
  }
}

#[test]
fn context_records_are_metadata_not_final_messages() {
  for record in [
    json!({"type":"turn_context","payload":{"turn_id":"turn-1","model":"model","effort":"low"}}),
    json!({"type":"world_state","payload":{"full":false,"state":{"opaque":true}}}),
    json!({"type":"inter_agent_communication_metadata","payload":{"trigger_turn":false}}),
    json!({"type":"compacted","payload":{"message":"context summary","replacement_history":[]}}),
    json!({"type":"event_msg","payload":{"type":"context_compacted"}}),
    json!({"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":2}}),
  ] {
    let events = line(&mut normalizer(), record.clone());
    assert!(
      matches!(&events[..], [AgentEvent::Metadata(event)] if event.native == record),
      "{record}"
    );
  }
}

#[test]
fn malformed_context_and_future_events_stay_unknown() {
  for record in [
    json!({"type":"turn_context","payload":{}}),
    json!({"type":"world_state","payload":{"full":true}}),
    json!({"type":"compacted","payload":{}}),
    json!({"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":-1}}),
    json!({"type":"event_msg","payload":{"type":"future_event","ignorable":true}}),
  ] {
    assert!(
      matches!(&line(&mut normalizer(), record.clone())[..], [AgentEvent::Unknown(_)]),
      "{record}"
    );
  }
}

#[test]
fn historical_subagent_filter_applies_before_accounting_and_context() {
  let mut normalizer = CodexNormalizer::new_historical();
  line(
    &mut normalizer,
    json!({"type":"session_meta","payload":{"id":"child",
    "source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}}}),
  );
  assert!(line(&mut normalizer, token_count(35)).is_empty());
  assert!(
    line(
      &mut normalizer,
      json!({"type":"world_state","payload":{"full":true,"state":{}}})
    )
    .is_empty()
  );
  assert!(
    line(
      &mut normalizer,
      json!({"type":"inter_agent_communication_metadata","payload":{"trigger_turn":true}})
    )
    .is_empty()
  );
  let events = line(&mut normalizer, token_count(35));
  assert!(matches!(&events[..], [AgentEvent::Usage(event)] if event.session_id.as_deref() == Some("child")));
}

#[test]
fn rollback_and_compaction_reset_snapshot_deduplication() {
  for record in [
    json!({"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}),
    json!({"type":"compacted","payload":{"message":"summary"}}),
  ] {
    let mut normalizer = normalizer();
    line(&mut normalizer, token_count(35));
    line(&mut normalizer, record);
    assert!(matches!(
      &line(&mut normalizer, token_count(35))[..],
      [AgentEvent::Usage(_)]
    ));
  }
}
