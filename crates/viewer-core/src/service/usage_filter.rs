//! Classify usage against the complete snapshot, never a loaded page or a
//! trajectory fragment. Accounting kinds remain distinct; no totals are added.

use std::collections::HashSet;
use tokn_session_core::{AgentEvent, LifecycleScope, MessageDelivery, Phase, Role, UsageKind};

#[derive(Default)]
struct TurnUsage<'a> {
  turn_id: Option<&'a str>,
  // Usage can supply an ID without an authoritative turn-start boundary.
  explicit: bool,
  // A final reply makes terminal accounting visible; a provider may still
  // append accounting before its explicit Finished boundary closes the turn.
  complete: bool,
  closed: bool,
  latest: [Option<usize>; 3],
}

impl TurnUsage<'_> {
  fn finish(&mut self, intermediate: &mut HashSet<usize>) {
    self.complete = true;
    for index in self.latest.iter().flatten() {
      intermediate.remove(index);
    }
  }

  fn record(&mut self, kind: UsageKind, index: usize, intermediate: &mut HashSet<usize>) {
    let slot = match kind {
      UsageKind::ModelCall => 0,
      UsageKind::OperationTotal => 1,
      UsageKind::SessionSnapshot => 2,
    };
    if let Some(previous) = self.latest[slot].replace(index) {
      intermediate.insert(previous);
    }
    if !self.complete {
      intermediate.insert(index);
    }
  }
}

pub(super) fn intermediate_usage(events: &[AgentEvent]) -> HashSet<usize> {
  let mut intermediate = HashSet::new();
  let mut turn: Option<TurnUsage<'_>> = None;
  for (index, event) in events.iter().enumerate() {
    if event.is_hidden() {
      continue;
    }
    match event {
      AgentEvent::SessionStarted(_) => {
        finish(&mut turn, &mut intermediate);
        turn = None;
      }
      AgentEvent::Lifecycle(lifecycle) if matches!(lifecycle.scope, LifecycleScope::Turn) => {
        let id = Some(lifecycle.turn_id.as_str()).filter(|id| !id.is_empty());
        match lifecycle.phase {
          Phase::Started => {
            if let Some(current) = &mut turn {
              if current.turn_id == id && !current.closed {
                current.explicit = true;
                continue;
              }
            }
            finish(&mut turn, &mut intermediate);
            turn = Some(TurnUsage {
              turn_id: id,
              explicit: true,
              ..Default::default()
            });
          }
          Phase::Finished => {
            // Late completion from an older turn must not close current work.
            if turn
              .as_ref()
              .is_none_or(|current| current.turn_id.is_none() || current.turn_id == id)
            {
              let current = turn.get_or_insert_with(TurnUsage::default);
              current.turn_id = id;
              current.closed = true;
              current.finish(&mut intermediate);
            }
          }
          _ => {}
        }
      }
      AgentEvent::Message(message) if message.role == Role::User => {
        // Explicit starts may precede the prompt; injected input can also
        // arrive during an active turn. Neither creates a second turn here.
        if turn.as_ref().is_some_and(|current| current.explicit && !current.closed) {
          continue;
        }
        finish(&mut turn, &mut intermediate);
        turn = Some(TurnUsage::default());
      }
      AgentEvent::Message(message) if message.role == Role::Assistant => {
        if turn.as_ref().is_some_and(|current| current.closed) {
          turn = None;
        }
        let current = turn.get_or_insert_with(TurnUsage::default);
        if message.delivery == MessageDelivery::Final {
          current.finish(&mut intermediate);
        } else {
          current.complete = false;
          intermediate.extend(current.latest.iter().flatten().copied());
        }
      }
      AgentEvent::Usage(usage) => {
        if let Some(current) = &mut turn {
          // Unrelated/late accounting is ambiguous, so leave it inspectable.
          if current
            .turn_id
            .zip(usage.turn_id.as_deref())
            .is_some_and(|(active, usage)| active != usage)
          {
            continue;
          }
          current.turn_id = current.turn_id.or(usage.turn_id.as_deref().filter(|id| !id.is_empty()));
          current.record(usage.kind, index, &mut intermediate);
        }
      }
      _ => {}
    }
  }
  intermediate
}

fn finish(turn: &mut Option<TurnUsage<'_>>, intermediate: &mut HashSet<usize>) {
  if let Some(current) = turn {
    current.finish(intermediate);
  }
}
