use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tempfile::TempDir;
use tokn_session_codex::CodexSessionSource;
use tokn_session_core::{AgentEvent, LoadedSessionRecords, SessionHistoryStatus};

fn meta(id: &str, ordinal: u64, base: Option<Value>) -> Value {
  json!({"type":"session_meta","ordinal":ordinal,"payload":{
    "id":id,"history_mode":"paginated","history_base":base,
    "timestamp":"2026-09-18T00:00:00Z","cwd":"/test"
  }})
}

fn message(id: &str, ordinal: u64, text: &str) -> Value {
  json!({"type":"event_msg","ordinal":ordinal,"payload":{
    "type":"item_completed","thread_id":id,"turn_id":format!("turn-{ordinal}"),
    "item":{"type":"UserMessage","id":format!("message-{ordinal}"),"content":[{"type":"text","text":text}]}
  }})
}

fn write(root: &Path, name: &str, rows: &[Value]) -> PathBuf {
  let path = root.join(name);
  fs::create_dir_all(path.parent().unwrap()).unwrap();
  fs::write(&path, rows.iter().map(|row| format!("{row}\n")).collect::<String>()).unwrap();
  path
}

fn base(id: &str, rows: &[Value]) -> Value {
  json!({"thread_id":id,"end_ordinal_exclusive":rows.last().unwrap()["ordinal"].as_u64().unwrap()+1,
    "end_byte_offset":rows.iter().map(|row| format!("{row}\n").len() as u64).sum::<u64>()})
}

fn load(source: &CodexSessionSource, path: &Path) -> LoadedSessionRecords {
  source.load_session_records_path(path, true, 1024 * 1024).unwrap()
}

fn texts(loaded: &LoadedSessionRecords) -> Vec<String> {
  loaded
    .records
    .iter()
    .flat_map(|record| &record.events)
    .filter_map(|event| match event {
      AgentEvent::Message(message) => Some(message.text.clone()),
      _ => None,
    })
    .collect()
}

#[test]
fn same_thread_continuation_keeps_prefix_excludes_rolled_back_suffix_and_appends_stably() {
  let root = TempDir::new().unwrap();
  let source = CodexSessionSource::new(Some(root.path().into()));
  let prefix = vec![meta("thread", 0, None), message("thread", 1, "previous message")];
  let mut original = prefix.clone();
  original.push(message("thread", 2, "rolled back message"));
  let original_path = write(root.path(), "old/segment.jsonl", &original);
  let current_rows = vec![
    meta("thread", 2, Some(base("thread", &prefix))),
    message("thread", 3, "new message"),
  ];
  let current = write(root.path(), "new/segment.jsonl", &current_rows);
  let loaded = load(&source, &current);
  assert_eq!(texts(&loaded), ["previous message", "new message"]);
  assert_eq!(loaded.reference.path, current);
  assert_eq!(loaded.reference.message_count, 2);
  assert_eq!(
    loaded
      .records
      .iter()
      .flat_map(|record| &record.events)
      .filter(|event| matches!(event, AgentEvent::SessionStarted(_)))
      .count(),
    1
  );
  assert_eq!(
    source
      .history_segments(&current)
      .unwrap()
      .iter()
      .map(|segment| &segment.path)
      .collect::<Vec<_>>(),
    [&original_path, &current]
  );

  let before = loaded
    .records
    .iter()
    .map(|record| record.record_id.clone())
    .collect::<Vec<_>>();
  let mut appended = current_rows;
  appended.push(message("thread", 4, "next message"));
  write(root.path(), "new/segment.jsonl", &appended);
  let updated = load(&source, &current);
  assert_eq!(texts(&updated), ["previous message", "new message", "next message"]);
  assert_eq!(
    updated.records[..before.len()]
      .iter()
      .map(|record| &record.record_id)
      .collect::<Vec<_>>(),
    before.iter().collect::<Vec<_>>()
  );
  assert_eq!(
    source
      .load_session_path(&current)
      .unwrap()
      .events
      .iter()
      .filter(|event| matches!(event, AgentEvent::Message(_)))
      .count(),
    3
  );
}

#[test]
fn nested_cross_thread_fork_retains_requested_owner_and_exact_bounds() {
  let root = TempDir::new().unwrap();
  let source = CodexSessionSource::new(Some(root.path().into()));
  let oldest = vec![meta("parent", 0, None), message("parent", 1, "parent history")];
  write(root.path(), "archived/parent.jsonl", &oldest);
  let middle = vec![
    meta("middle", 2, Some(base("parent", &oldest))),
    message("middle", 3, "middle history"),
  ];
  write(root.path(), "middle.jsonl", &middle);
  let head = write(
    root.path(),
    "child.jsonl",
    &[
      meta("child", 4, Some(base("middle", &middle))),
      message("child", 5, "child history"),
    ],
  );
  let loaded = load(&source, &head);
  assert_eq!(texts(&loaded), ["parent history", "middle history", "child history"]);
  for event in loaded.records.iter().flat_map(|record| &record.events) {
    if let AgentEvent::Message(message) = event {
      assert_eq!(message.session_id.as_deref(), Some("child"));
    }
  }
}

#[test]
fn subagent_inherited_prefix_remains_filtered_until_its_own_trigger() {
  let root = TempDir::new().unwrap();
  let source = CodexSessionSource::new(Some(root.path().into()));
  let parent = vec![
    meta("parent", 0, None),
    json!({"type":"inter_agent_communication_metadata","ordinal":1,"payload":{"trigger_turn":true}}),
    message("parent", 2, "parent history"),
  ];
  write(root.path(), "parent.jsonl", &parent);
  let mut owner = meta("child", 3, Some(base("parent", &parent)));
  owner["payload"]["source"] = json!({"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}});
  owner["payload"]["parent_thread_id"] = json!("parent");
  let head = write(
    root.path(),
    "child.jsonl",
    &[
      owner,
      json!({"type":"inter_agent_communication_metadata","ordinal":4,"payload":{"trigger_turn":true}}),
      message("child", 5, "child history"),
    ],
  );
  let loaded = load(&source, &head);
  assert_eq!(texts(&loaded), ["child history"]);
  assert_eq!(loaded.history_status, SessionHistoryStatus::FilteredSubagent);
}

#[test]
fn unavailable_ambiguous_and_invalid_prefixes_fail_without_publishing_partial_history() {
  let root = TempDir::new().unwrap();
  let source = CodexSessionSource::new(Some(root.path().into()));
  let prefix = vec![meta("thread", 0, None), message("thread", 1, "history")];
  let pointer = base("thread", &prefix);
  let rows = [meta("thread", 2, Some(pointer.clone())), message("thread", 3, "new")];
  let head = write(root.path(), "head.jsonl", &rows);
  assert!(source.history_segments(&head).unwrap_err().contains("unavailable"));
  let old = write(root.path(), "old.jsonl", &prefix);
  let duplicate = write(root.path(), "duplicate.jsonl", &prefix);
  assert!(source.history_segments(&head).unwrap_err().contains("ambiguous"));
  fs::remove_file(duplicate).unwrap();
  for invalid_offset in [
    pointer["end_byte_offset"].as_u64().unwrap() - 1,
    pointer["end_byte_offset"].as_u64().unwrap() + 1,
  ] {
    let mut invalid = pointer.clone();
    invalid["end_byte_offset"] = json!(invalid_offset);
    write(root.path(), "head.jsonl", &[meta("thread", 2, Some(invalid))]);
    assert!(source.history_segments(&head).is_err());
  }
  // A matching final ordinal cannot excuse a malformed earlier prefix.
  let bytes = fs::read(&old).unwrap();
  let mut broken = bytes.clone();
  broken[1] = b'!';
  fs::write(&old, broken).unwrap();
  write(root.path(), "head.jsonl", &rows);
  assert!(source.load_session_records_path(&head, false, 1024 * 1024).is_err());
}

#[test]
fn explicit_root_isolation_and_combined_byte_limit_are_enforced() {
  let root = TempDir::new().unwrap();
  let prefix = vec![meta("thread", 0, None), message("thread", 1, "history")];
  write(root.path(), "outside/old.jsonl", &prefix);
  let head = write(
    root.path(),
    "inside/head.jsonl",
    &[
      meta("thread", 2, Some(base("thread", &prefix))),
      message("thread", 3, "new"),
    ],
  );
  assert!(
    CodexSessionSource::new(Some(root.path().join("inside")))
      .history_segments(&head)
      .is_err()
  );
  let source = CodexSessionSource::new(Some(root.path().into()));
  let size = fs::metadata(&head).unwrap().len() as usize;
  assert!(
    source
      .load_session_records_path(&head, false, size)
      .unwrap_err()
      .contains("size limit")
  );
  assert_eq!(texts(&load(&source, &head)), ["history", "new"]);
}

#[test]
fn incomplete_active_tail_is_deferred_and_corrupt_complete_tail_is_an_error() {
  let root = TempDir::new().unwrap();
  let source = CodexSessionSource::new(Some(root.path().into()));
  let prefix = vec![meta("thread", 0, None), message("thread", 1, "history")];
  write(root.path(), "old.jsonl", &prefix);
  let head = write(
    root.path(),
    "head.jsonl",
    &[meta("thread", 2, Some(base("thread", &prefix)))],
  );
  let mut bytes = fs::read(&head).unwrap();
  bytes.extend(b"{\"partial\":");
  fs::write(&head, &bytes).unwrap();
  assert_eq!(texts(&load(&source, &head)), ["history"]);
  bytes.push(b'\n');
  fs::write(&head, bytes).unwrap();
  assert!(source.load_session_records_path(&head, false, 1024 * 1024).is_err());
}
