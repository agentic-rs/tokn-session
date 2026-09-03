//! Compaction observations, not conversation replies or turn completion.
use crate::{AgentEvent, Provider};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionState {
  Requested,
  Started,
  SummaryGenerated,
  Completed,
  Failed,
  Interrupted,
  Skipped,
}

impl CompactionState {
  pub fn label(self) -> &'static str {
    match self {
      Self::Requested => "Compaction requested",
      Self::Started => "Compacting…",
      Self::SummaryGenerated => "Compaction summary generated",
      Self::Completed => "Context compacted",
      Self::Failed => "Compaction failed",
      Self::Interrupted => "Compaction interrupted",
      Self::Skipped => "Compaction skipped",
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTokenScope {
  ContextBefore,
  ContextAfter,
  ReplacedContext,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactionTokens {
  pub scope: CompactionTokenScope,
  pub tokens: u64,
  /// None means the source does not establish whether this is an estimate.
  pub estimated: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompactionContext {
  pub first_kept_entry_id: Option<String>,
  pub last_summarized_entry_id: Option<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub replaced_entry_ids: Vec<String>,
  pub previous_window_id: Option<String>,
  pub window_id: Option<String>,
  pub window_number: Option<u64>,
  pub summarized_message_count: Option<u64>,
  pub kept_message_count: Option<u64>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub summary_message_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactionEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  /// Provider/session-scoped operation identity; absent observations are never
  /// guessed into the same operation by matching text or timestamps.
  pub compaction_id: Option<String>,
  pub state: CompactionState,
  pub timestamp: Option<String>,
  pub turn_id: Option<String>,
  pub trigger: Option<String>,
  pub reason: Option<String>,
  /// Native timing category (e.g. pre_request), not lifecycle state.
  pub provider_phase: Option<String>,
  pub summary: Option<String>,
  #[serde(default)]
  pub summary_opaque: bool,
  pub model_provider: Option<String>,
  pub model_id: Option<String>,
  #[serde(default)]
  pub context: CompactionContext,
  #[serde(default)]
  pub measurements: Vec<CompactionTokens>,
  #[serde(default)]
  pub source_refs: Vec<String>,
}

impl CompactionEvent {
  pub fn new(provider: Provider, session_id: Option<String>, state: CompactionState) -> Self {
    Self {
      provider,
      session_id,
      state,
      compaction_id: None,
      timestamp: None,
      turn_id: None,
      trigger: None,
      reason: None,
      provider_phase: None,
      summary: None,
      summary_opaque: false,
      model_provider: None,
      model_id: None,
      context: CompactionContext::default(),
      measurements: Vec::new(),
      source_refs: Vec::new(),
    }
  }

  pub fn tokens(&mut self, scope: CompactionTokenScope, tokens: u64, estimated: Option<bool>) {
    self.measurements.retain(|item| item.scope != scope);
    self.measurements.push(CompactionTokens {
      scope,
      tokens,
      estimated,
    });
  }

  fn update(&mut self, next: &Self) {
    // Database rows can contain a finished operation before its later summary
    // row. Enrichment must not reopen a terminal operation.
    let terminal = |state| {
      matches!(
        state,
        CompactionState::Completed | CompactionState::Failed | CompactionState::Interrupted | CompactionState::Skipped
      )
    };
    if !terminal(self.state) || terminal(next.state) {
      self.state = next.state;
      if next.timestamp.is_some() {
        self.timestamp = next.timestamp.clone();
      }
      if next.reason.is_some() {
        self.reason = next.reason.clone();
      }
    } else if self.reason.is_none() {
      self.reason = next.reason.clone();
    }
    macro_rules! replace_present { ($($field:ident),*) => { $(if next.$field.is_some() { self.$field = next.$field.clone(); })* }; }
    replace_present!(turn_id, trigger, provider_phase, summary, model_provider, model_id);
    self.summary_opaque |= next.summary_opaque;
    macro_rules! context_present { ($($field:ident),*) => { $(if next.context.$field.is_some() { self.context.$field = next.context.$field.clone(); })* }; }
    context_present!(
      first_kept_entry_id,
      last_summarized_entry_id,
      previous_window_id,
      window_id,
      window_number,
      summarized_message_count,
      kept_message_count
    );
    if !next.context.replaced_entry_ids.is_empty() {
      self.context.replaced_entry_ids = next.context.replaced_entry_ids.clone();
    }
    if !next.context.summary_message_ids.is_empty() {
      self.context.summary_message_ids = next.context.summary_message_ids.clone();
    }
    for item in &next.measurements {
      self.tokens(item.scope, item.tokens, item.estimated);
    }
    for reference in &next.source_refs {
      if !self.source_refs.contains(reference) {
        self.source_refs.push(reference.clone());
      }
    }
  }
}

#[derive(Clone, Debug)]
pub struct CompactionOperation {
  pub event: CompactionEvent,
  pub source_event_indices: Vec<usize>,
}

/// One display operation per known identity; the underlying append-only event
/// stream remains untouched. The first source index is a stable detail key.
pub fn compaction_operations(events: &[AgentEvent]) -> Vec<CompactionOperation> {
  let mut positions = HashMap::new();
  let mut operations: Vec<CompactionOperation> = Vec::new();
  for (index, event) in events.iter().enumerate() {
    let AgentEvent::Compaction(event) = event else {
      continue;
    };
    let key = event
      .compaction_id
      .as_ref()
      .filter(|id| !id.is_empty())
      .map(|id| (event.provider, event.session_id.clone(), id.clone()));
    if let Some(position) = key.as_ref().and_then(|key| positions.get(key)).copied() {
      let operation: &mut CompactionOperation = &mut operations[position];
      operation.event.update(event);
      operation.source_event_indices.push(index);
    } else {
      if let Some(key) = key {
        positions.insert(key, operations.len());
      }
      operations.push(CompactionOperation {
        event: event.clone(),
        source_event_indices: vec![index],
      });
    }
  }
  operations
}

#[cfg(test)]
mod tests {
  use super::*;

  fn observation(state: CompactionState) -> CompactionEvent {
    let mut event = CompactionEvent::new(Provider::Dsh, Some("session".into()), state);
    event.compaction_id = Some("operation".into());
    event
  }

  #[test]
  fn correlated_observations_keep_one_stable_identity_and_enrich_terminal_state() {
    let mut start = observation(CompactionState::Started);
    start.source_refs.push("1".into());
    let mut summary = observation(CompactionState::SummaryGenerated);
    summary.summary = Some("context summary".into());
    summary.tokens(CompactionTokenScope::ReplacedContext, 100, Some(true));
    summary.source_refs.push("2".into());
    let mut end = observation(CompactionState::Failed);
    end.reason = Some("checkpoint failed".into());
    let events = vec![start, summary.clone(), end, summary]
      .into_iter()
      .map(AgentEvent::Compaction)
      .collect::<Vec<_>>();
    let operations = compaction_operations(&events);
    assert_eq!(operations.len(), 1);
    let operation = &operations[0];
    assert_eq!(operation.source_event_indices, [0, 1, 2, 3]);
    assert_eq!(operation.event.state, CompactionState::Failed);
    assert_eq!(operation.event.summary.as_deref(), Some("context summary"));
    assert_eq!(operation.event.reason.as_deref(), Some("checkpoint failed"));
    assert_eq!(operation.event.measurements.len(), 1);
    assert_eq!(operation.event.source_refs, ["1", "2"]);
    let serialized = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(serialized["type"], "compaction");
    assert!(serialized.get("native").is_none());
    let _: AgentEvent = serde_json::from_value(serialized).unwrap();
  }

  #[test]
  fn identities_are_provider_and_session_scoped_and_absence_never_deduplicates() {
    let first = observation(CompactionState::Completed);
    let mut other_session = first.clone();
    other_session.session_id = Some("other".into());
    let mut other_provider = first.clone();
    other_provider.provider = Provider::Pi;
    let mut anonymous = first.clone();
    anonymous.compaction_id = None;
    let events = vec![first, other_session, other_provider, anonymous.clone(), anonymous]
      .into_iter()
      .map(AgentEvent::Compaction)
      .collect::<Vec<_>>();
    assert_eq!(compaction_operations(&events).len(), 5);
  }
}
