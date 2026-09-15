use super::*;
use tokn_session_core::AgentCommunication;

fn communication() -> AgentEvent {
  let mut event = agent_activity("parent", Some("/root"));
  let AgentEvent::AgentActivity(activity) = &mut event else {
    unreachable!()
  };
  activity.kind = "messaged".into();
  activity.actor_agent_path = Some("/root/reviewer".into());
  activity.communication = Some(AgentCommunication {
    text: Some("## Review\n\n- **Ready**\n".into()),
    has_encrypted_content: true,
    trigger_turn: Some(false),
  });
  event
}

fn headers() -> Vec<SessionHeader> {
  let mut parent = session_header("parent", None, "/project", "2026-09-14T00:00:00Z");
  parent.agent_path = Some("/root".into());
  let mut child = session_header("child", Some("parent"), "/project", "2026-09-14T00:01:00Z");
  child.agent_path = Some("/root/reviewer".into());
  vec![parent, child]
}

fn service(headers: Vec<SessionHeader>, owner: &str, event: AgentEvent) -> ViewerService {
  service_with_indexed_headers(
    Arc::new(FakeRepository {
      listings: HashMap::new(),
      loaded: Mutex::new(Some(loaded_session_for(owner, vec![event]))),
    }),
    vec![(ViewerProvider::Codex, headers)],
  )
}

#[test]
fn communication_detail_is_readable_without_native_and_summary_has_no_body() {
  let event = communication();
  let summary = event_summary(&[event.clone()], 0, &event);
  let card = summary.agent_activity.as_ref().unwrap();
  assert_eq!(card.actor_agent_path.as_deref(), Some("/root/reviewer"));
  assert!(card.communication.as_ref().unwrap().has_text);
  assert!(card.communication.as_ref().unwrap().has_encrypted_content);
  assert_eq!(card.communication.as_ref().unwrap().trigger_turn, Some(false));
  assert!(!serde_json::to_string(&summary).unwrap().contains("**Ready**"));
  let detail = event_detail("event-0".into(), &event, &[event.clone()], 0).unwrap();
  assert_eq!(detail.event["communication"]["text"], "## Review\n\n- **Ready**\n");
  assert!(detail.native.is_none());
}

#[test]
fn sender_links_use_unique_paths_within_the_current_tree() {
  let mut source_headers = headers();
  let mut unrelated = session_header("unrelated", None, "/project", "2026-09-14T00:02:00Z");
  unrelated.agent_path = Some("/root/reviewer".into());
  source_headers.push(unrelated);
  let service = service(source_headers, "parent", communication());
  let page = service
    .load_trajectory_event_page(LoadTrajectoryEventPageRequest {
      session_key: key_for_header("parent"),
      trajectory_key: encode_trajectory_key(0),
      cursor: None,
      offset: None,
      direction: PageDirection::Forward,
      limit: None,
    })
    .unwrap();
  let activity = page.events[0].agent_activity.as_ref().unwrap();
  assert_eq!(activity.actor.as_ref().unwrap().session_id, "child");
  assert!(activity.target.is_none(), "self is not a direct-child link");
}

#[test]
fn ambiguous_or_conflicting_sender_identity_remains_unlinked() {
  let mut source_headers = headers();
  let mut duplicate = source_headers[1].clone();
  duplicate.id = "other-child".into();
  duplicate.path = PathBuf::from("/fixtures/other-child.jsonl");
  source_headers.push(duplicate);
  let event = communication();
  let service = service(source_headers, "parent", event.clone());
  let targets = service.delegation_targets_for_parent(&decode_session_key(&key_for_header("parent")).unwrap());
  let AgentEvent::AgentActivity(mut activity) = event else {
    unreachable!()
  };
  assert!(targets.sender(&activity).is_none());
  activity.actor_session_id = Some("child".into());
  assert_eq!(targets.sender(&activity).unwrap().session_id, "child");
  activity.actor_agent_path = Some("/root/different".into());
  assert!(targets.sender(&activity).is_none());
  activity.actor_session_id = Some("missing".into());
  activity.actor_agent_path = Some("/root/reviewer".into());
  assert!(targets.sender(&activity).is_none());
}

#[test]
fn child_communication_can_link_its_recorded_parent_sender() {
  let AgentEvent::AgentActivity(mut activity) = communication() else {
    unreachable!()
  };
  activity.actor_agent_path = Some("/root".into());
  let event = AgentEvent::AgentActivity(activity.clone());
  let service = service(headers(), "child", event);
  let targets = service.delegation_targets_for_parent(&decode_session_key(&key_for_header("child")).unwrap());
  assert_eq!(targets.sender(&activity).unwrap().session_id, "parent");
}

#[test]
fn current_session_remains_part_of_sender_path_ambiguity() {
  let mut source_headers = headers();
  source_headers[0].agent_path = source_headers[1].agent_path.clone();
  let AgentEvent::AgentActivity(mut activity) = communication() else {
    unreachable!()
  };
  let service = service(source_headers, "parent", AgentEvent::AgentActivity(activity.clone()));
  let targets = service.delegation_targets_for_parent(&decode_session_key(&key_for_header("parent")).unwrap());
  assert!(
    targets.sender(&activity).is_none(),
    "self must not hide a duplicate path"
  );
  activity.actor_session_id = Some("parent".into());
  assert!(
    targets.sender(&activity).is_none(),
    "a unique explicit self ID has no navigation"
  );
  activity.actor_session_id = Some("child".into());
  assert_eq!(targets.sender(&activity).unwrap().session_id, "child");
}

#[test]
fn indexed_sender_paths_preserve_raw_identity_across_whitespace_sanitizing() {
  let mut source_headers = headers();
  source_headers[1].agent_path = Some(" /root/reviewer \n".into());
  let AgentEvent::AgentActivity(mut activity) = communication() else {
    unreachable!()
  };
  let service = service(source_headers, "parent", AgentEvent::AgentActivity(activity.clone()));
  let targets = service.delegation_targets_for_parent(&decode_session_key(&key_for_header("parent")).unwrap());
  assert!(
    targets.sender(&activity).is_none(),
    "a display-normalized path is not an identity"
  );
  activity.actor_session_id = Some("child".into());
  assert!(
    targets.sender(&activity).is_none(),
    "explicit ID cannot override a conflicting raw path"
  );
  activity.actor_agent_path = Some(" /root/reviewer \n".into());
  let sender = targets.sender(&activity).unwrap();
  assert_eq!(sender.session_id, "child");
  assert_eq!(
    sender.agent_path.as_deref(),
    Some("/root/reviewer"),
    "display stays sanitized"
  );
}

#[test]
fn indexed_long_sender_paths_match_raw_identity_despite_identical_display_prefixes() {
  let prefix = format!("/root/{}", "a".repeat(MAX_AGENT_IDENTITY_CHARS));
  let first_path = format!("{prefix}_first");
  let second_path = format!("{prefix}_second");
  let mut source_headers = headers();
  source_headers[1].agent_path = Some(first_path.clone());
  let mut sibling = session_header("sibling", Some("parent"), "/project", "2026-09-14T00:02:00Z");
  sibling.agent_path = Some(second_path.clone());
  source_headers.push(sibling);
  let AgentEvent::AgentActivity(mut activity) = communication() else {
    unreachable!()
  };
  let service = service(source_headers, "parent", AgentEvent::AgentActivity(activity.clone()));
  let targets = service.delegation_targets_for_parent(&decode_session_key(&key_for_header("parent")).unwrap());
  activity.actor_agent_path = Some(first_path);
  let first = targets.sender(&activity).unwrap();
  assert_eq!(first.session_id, "child");
  assert_eq!(
    first.agent_path.as_ref().unwrap().chars().count(),
    MAX_AGENT_IDENTITY_CHARS
  );
  activity.actor_agent_path = Some(second_path);
  let second = targets.sender(&activity).unwrap();
  assert_eq!(second.session_id, "sibling");
  assert_eq!(
    first.agent_path, second.agent_path,
    "display labels share the truncated prefix"
  );
  activity.actor_agent_path = first.agent_path;
  assert!(
    targets.sender(&activity).is_none(),
    "a truncated display path is not an identity"
  );
}
