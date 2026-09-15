use super::*;
use crate::model::AgentCommunicationCardSummary;

#[derive(Default)]
pub(super) struct ActivityTargets {
  pub(super) direct_children: HashMap<String, SessionSummary>,
  related: HashMap<String, RelatedSession>,
  current_session_id: Option<String>,
}

struct RelatedSession {
  // Presentation paths may be shortened or sanitized. Identity matching must
  // use the original header value, including when an explicit ID is supplied.
  agent_path: Option<String>,
  summary: SessionSummary,
}

impl ActivityTargets {
  pub(super) fn sender(&self, activity: &AgentActivity) -> Option<SessionSummary> {
    activity.communication.as_ref()?;
    let path = activity.actor_agent_path.as_deref();
    if let Some(id) = activity.actor_session_id.as_deref() {
      let session = self.related.get(id)?;
      // Conflicting supplied identities cannot safely become navigation.
      return path
        .is_none_or(|path| session.agent_path.as_deref() == Some(path))
        .then(|| self.navigation_target(session))
        .flatten();
    }
    let path = path?;
    let mut matches = self
      .related
      .values()
      .filter(|session| session.agent_path.as_deref() == Some(path));
    let session = matches.next()?;
    matches
      .next()
      .is_none()
      .then(|| self.navigation_target(session))
      .flatten()
  }

  fn navigation_target(&self, session: &RelatedSession) -> Option<SessionSummary> {
    (self.current_session_id.as_deref() != Some(session.summary.session_id.as_str())).then(|| session.summary.clone())
  }
}

impl ViewerService {
  /// Resolve identities only inside this canonical session's relation tree.
  /// Identical agent paths in other root tasks are unrelated; duplicate paths
  /// inside this tree remain ambiguous unless an explicit session ID agrees.
  pub(super) fn delegation_targets_for_parent(&self, locator: &SessionLocator) -> ActivityTargets {
    let Ok(Some(inventory)) = self.indexed_session_inventory(locator.provider) else {
      return ActivityTargets::default();
    };
    let mut ignored_errors = Vec::new();
    let relations = session_relation_index(locator.provider, inventory.headers, &mut ignored_errors);
    let attention = session_relation_attention(locator.provider, &relations, &inventory.direct_attention);
    let Some(current) = relations
      .headers
      .iter()
      .position(|header| locator_for_header(locator.provider, header) == *locator)
    else {
      return ActivityTargets::default();
    };
    let tree_root = relation_root(&relations.parent_indices, current);
    let mut targets = ActivityTargets {
      current_session_id: Some(locator.session_id.clone()),
      ..Default::default()
    };
    for (index, header) in relations.headers.into_iter().enumerate() {
      if relation_root(&relations.parent_indices, index) != tree_root {
        continue;
      }
      let is_direct_child = relations.parent_indices[index] == Some(current);
      let agent_path = header.agent_path.clone();
      let Ok(summary) = session_summary_with_child_count(
        locator.provider,
        header,
        relations.child_counts[index],
        relations.parent_indices[index].is_some(),
        attention[index],
      ) else {
        continue;
      };
      if is_direct_child {
        targets
          .direct_children
          .insert(summary.session_id.clone(), summary.clone());
      }
      // Keep self in identity matching so a repeated path cannot falsely look
      // unique. Self navigation is suppressed only after resolving identity.
      targets
        .related
        .insert(summary.session_id.clone(), RelatedSession { agent_path, summary });
    }
    targets
  }
}

fn relation_root(parents: &[Option<usize>], mut index: usize) -> usize {
  // session_relation_index has already rejected cycles and unresolved edges.
  while let Some(parent) = parents[index] {
    index = parent;
  }
  index
}

pub(super) fn agent_activity_card_summary(
  activity: &AgentActivity,
  targets: &ActivityTargets,
) -> AgentActivityCardSummary {
  let label = |value: Option<&str>| value.and_then(|value| normalize_one_line_text(value, MAX_AGENT_IDENTITY_CHARS));
  AgentActivityCardSummary {
    kind: label(Some(&activity.kind)).unwrap_or_else(|| "activity".to_string()),
    event_id: label(activity.event_id.as_deref()),
    target_session_id: label(activity.target_session_id.as_deref()),
    target_agent_path: label(activity.target_agent_path.as_deref()),
    target: activity
      .target_session_id
      .as_deref()
      .and_then(|id| targets.direct_children.get(id))
      .cloned(),
    actor_session_id: label(activity.actor_session_id.as_deref()),
    actor_agent_path: label(activity.actor_agent_path.as_deref()),
    actor: targets.sender(activity),
    communication: activity
      .communication
      .as_ref()
      .map(|communication| AgentCommunicationCardSummary {
        has_text: communication.text.as_ref().is_some_and(|text| !text.trim().is_empty()),
        has_encrypted_content: communication.has_encrypted_content,
        trigger_turn: communication.trigger_turn,
      }),
  }
}
