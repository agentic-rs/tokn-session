use super::*;
use crate::model::{CompactionCardSummary, CompactionTokenSummary};
use tokn_session_core::{CompactionOperation, compaction_operations};

pub(super) fn for_source(events: &[AgentEvent], index: usize) -> Option<CompactionOperation> {
  if !matches!(events.get(index), Some(AgentEvent::Compaction(_))) {
    return None;
  }
  compaction_operations(events)
    .into_iter()
    .find(|operation| operation.source_event_indices.first() == Some(&index))
}

pub(super) fn card(event: &AgentEvent) -> Option<CompactionCardSummary> {
  let AgentEvent::Compaction(event) = event else {
    return None;
  };
  Some(CompactionCardSummary {
    state: serialized_label(event.state).unwrap(),
    trigger: event.trigger.as_ref().map(|s| truncate(s.clone(), 120)),
    reason: event.reason.as_ref().map(|s| truncate(s.clone(), 500)),
    has_summary: event.summary.as_ref().is_some_and(|s| !s.is_empty()),
    summary_opaque: event.summary_opaque,
    measurements: event
      .measurements
      .iter()
      .map(|item| CompactionTokenSummary {
        scope: serialized_label(item.scope).unwrap(),
        tokens: item.tokens.to_string(),
        estimated: item.estimated,
      })
      .collect(),
  })
}

#[cfg(test)]
mod tests {
  use super::super::tests::{key_for, loaded_session, service_with_session};
  use super::*;
  use tokn_session_core::{CompactionEvent, CompactionState, CompactionTokenScope, LifecycleEvent, LifecycleScope};

  #[test]
  fn one_stable_card_aggregates_details_without_finishing_the_active_turn() {
    let mut compact = CompactionEvent::new(Provider::Codex, Some("fixture".into()), CompactionState::Started);
    compact.compaction_id = Some("op".into());
    let mut events = vec![
      AgentEvent::Lifecycle(LifecycleEvent {
        provider: Provider::Codex,
        session_id: Some("fixture".into()),
        turn_id: "turn".into(),
        step_id: None,
        scope: LifecycleScope::Turn,
        phase: Phase::Started,
        outcome: None,
        timestamp: None,
        native: json!({}),
      }),
      AgentEvent::Compaction(compact.clone()),
    ];
    let page = |events: Vec<AgentEvent>| {
      service_with_session(loaded_session(events))
        .load_event_page(EventPageRequest {
          session_key: key_for("fixture"),
          cursor: None,
          offset: None,
          direction: PageDirection::Forward,
          limit: None,
        })
        .unwrap()
    };
    let initial = page(events.clone());
    assert_eq!(initial.events.len(), 2);
    assert_eq!(initial.events[1].title, "Compacting…");
    let key = initial.events[1].event_key.clone();
    compact.state = CompactionState::Completed;
    compact.summary = Some("## Summary\nretained decisions".into());
    compact.tokens(CompactionTokenScope::ContextBefore, u64::MAX, None);
    events.push(AgentEvent::Compaction(compact));
    let finished = page(events.clone());
    assert_eq!(finished.events.len(), 2);
    assert_eq!(finished.events[1].event_key, key);
    assert_eq!(finished.events[1].title, "Context compacted");
    assert_eq!(finished.events[0].trajectory.as_ref().unwrap().status, "working");
    assert_eq!(
      finished.events[1].compaction.as_ref().unwrap().measurements[0].tokens,
      u64::MAX.to_string()
    );
    let service = service_with_session(loaded_session(events));
    let detail = service
      .load_event_detail(LoadEventDetailRequest {
        session_key: key_for("fixture"),
        event_key: key,
      })
      .unwrap();
    assert_eq!(detail.event["summary"], "## Summary\nretained decisions");
    assert_eq!(detail.event["state"], "completed");
    assert!(detail.native.is_none());
    assert!(
      service
        .load_event_detail(LoadEventDetailRequest {
          session_key: key_for("fixture"),
          event_key: encode_event_key(2)
        })
        .is_err()
    );
  }
}
