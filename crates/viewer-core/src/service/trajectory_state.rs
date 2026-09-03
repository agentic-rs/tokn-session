use super::*;
use tokn_session_core::LifecycleScope;

pub(super) fn is_turn_start(entry: &TimelineEntry, events: &[AgentEvent]) -> bool {
  matches!(entry, TimelineEntry::Event { source_event_index }
    if matches!(&events[*source_event_index], AgentEvent::Lifecycle(l)
      if matches!(l.scope, LifecycleScope::Turn) && l.phase == Phase::Started))
}

pub(super) fn status(trajectory: &Trajectory, events: &[AgentEvent]) -> &'static str {
  let end = trajectory_source_event_indices(trajectory)
    .into_iter()
    .max()
    .unwrap_or(trajectory.start_source_event_index);
  let turn_id = active_turn_id(&events[..=end]);
  // A final reply or the next conversation boundary closes this work run.
  // Tool completion alone never closes a turn. Metadata does not reopen one.
  for event in &events[end + 1..] {
    if event.is_hidden() {
      continue;
    }
    match event {
      AgentEvent::Message(m) if m.role == Role::User || is_final_assistant_message(m.role, m.delivery) => {
        return "complete";
      }
      AgentEvent::Lifecycle(l) if matches!(l.scope, LifecycleScope::Turn) => {
        if l.phase == Phase::Started || (l.phase == Phase::Finished && turn_id.is_none_or(|id| id == l.turn_id)) {
          return "complete";
        }
      }
      _ => {}
    }
  }
  for event in events[..=end].iter().rev() {
    match event {
      AgentEvent::Lifecycle(l) if matches!(l.scope, LifecycleScope::Turn) => {
        if turn_id.is_some_and(|id| id != l.turn_id) {
          continue;
        }
        return if l.phase == Phase::Started {
          "working"
        } else if l.phase == Phase::Finished {
          "complete"
        } else {
          "unknown"
        };
      }
      AgentEvent::Message(m) if is_final_assistant_message(m.role, m.delivery) => break,
      AgentEvent::SessionStarted(_) => break,
      _ => {}
    }
  }
  "unknown"
}

pub(super) fn completion_timestamp<'a>(trajectory: &Trajectory, events: &'a [AgentEvent]) -> Option<&'a str> {
  let end = trajectory_source_event_indices(trajectory).into_iter().max()?;
  let turn_id = active_turn_id(&events[..=end]);
  for event in &events[end + 1..] {
    if event.is_hidden() {
      continue;
    }
    match event {
      AgentEvent::Message(m) if is_final_assistant_message(m.role, m.delivery) => return m.timestamp.as_deref(),
      AgentEvent::Message(m) if m.role == Role::User => return None,
      AgentEvent::Lifecycle(l) if matches!(l.scope, LifecycleScope::Turn) => {
        if l.phase != Phase::Started && turn_id.is_some_and(|id| id != l.turn_id) {
          continue;
        }
        return if l.phase == Phase::Finished {
          l.timestamp.as_deref()
        } else {
          None
        };
      }
      _ => {}
    }
  }
  None
}

fn active_turn_id(events: &[AgentEvent]) -> Option<&str> {
  for event in events.iter().rev() {
    match event {
      AgentEvent::Lifecycle(l) if matches!(l.scope, LifecycleScope::Turn) && l.phase == Phase::Started => {
        return Some(&l.turn_id);
      }
      AgentEvent::Message(m) if is_final_assistant_message(m.role, m.delivery) => break,
      AgentEvent::SessionStarted(_) => break,
      _ => {}
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::super::tests::{key_for, loaded_session, message_event_with_role, service_with_session};
  use super::*;
  use tokn_session_core::LifecycleEvent;

  fn boundary(phase: Phase) -> AgentEvent {
    AgentEvent::Lifecycle(LifecycleEvent {
      provider: Provider::Codex,
      session_id: Some("fixture".into()),
      turn_id: "t1".into(),
      step_id: None,
      scope: LifecycleScope::Turn,
      phase,
      outcome: None,
      native: json!({}),
      timestamp: Some("2026-09-03T00:00:00Z".into()),
    })
  }

  #[test]
  fn step_completion_and_a_late_other_turn_finish_do_not_close_active_work() {
    let mut other = boundary(Phase::Finished);
    if let AgentEvent::Lifecycle(l) = &mut other {
      l.turn_id = "older-turn".into();
    }
    let mut step = boundary(Phase::Finished);
    if let AgentEvent::Lifecycle(l) = &mut step {
      l.scope = LifecycleScope::Step;
    }
    let events = vec![boundary(Phase::Started), step, other];
    let TimelineEntry::Trajectory { trajectory } = &timeline_entries(&events)[0] else {
      panic!()
    };
    assert_eq!(status(trajectory, &events), "working");
  }

  #[test]
  fn active_turn_has_one_stable_trajectory_and_closes_on_finish() {
    let mut events = vec![
      boundary(Phase::Started),
      message_event_with_role("prompt", Role::User, MessageDelivery::Unspecified),
      message_event_with_role("working", Role::Assistant, MessageDelivery::Commentary),
    ];
    let get = |events: Vec<AgentEvent>| {
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
    let working = get(events.clone());
    assert_eq!(working.events.len(), 2);
    assert_eq!(working.events[1].trajectory.as_ref().unwrap().status, "working");
    events.push(boundary(Phase::Finished));
    let finished = get(events.clone());
    assert_eq!(finished.events[1].event_key, working.events[1].event_key);
    assert_eq!(finished.events[1].trajectory.as_ref().unwrap().status, "complete");
    events.push(boundary(Phase::Started));
    let next = get(events);
    assert_eq!(
      next.events.last().unwrap().trajectory.as_ref().unwrap().status,
      "working"
    );
  }

  #[test]
  fn final_reply_closes_work_but_unknown_history_is_not_assumed_running() {
    let mut events = vec![message_event_with_role(
      "progress",
      Role::Assistant,
      MessageDelivery::Commentary,
    )];
    let TimelineEntry::Trajectory { trajectory } = &timeline_entries(&events)[0] else {
      panic!()
    };
    assert_eq!(status(trajectory, &events), "unknown");
    events.push(message_event_with_role("done", Role::Assistant, MessageDelivery::Final));
    let TimelineEntry::Trajectory { trajectory } = &timeline_entries(&events)[0] else {
      panic!()
    };
    assert_eq!(status(trajectory, &events), "complete");
  }
}
