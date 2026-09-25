use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tempfile::TempDir;
use tokn_session_codex::{CodexHistoryReader, CodexSessionSource};
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
fn repeated_continuation_resolves_physical_segment_id_and_preserves_bounds() {
  const OWNER: &str = "01a0c8ab-3b96-7c40-adae-51c20a43c2cb";
  const SEGMENT: &str = "01a0c923-943a-7820-b93f-15e26e7e7cd4";
  let root = TempDir::new().unwrap();
  let source = CodexSessionSource::new(Some(root.path().into()));
  let original = vec![meta(OWNER, 0, None), message(OWNER, 1, "original")];
  write(root.path(), "original.jsonl", &original);
  let prefix = vec![
    meta(OWNER, 2, Some(base(OWNER, &original))),
    message(OWNER, 3, "retained"),
  ];
  let mut middle = prefix.clone();
  middle.push(message(OWNER, 4, "reverted"));
  let name = format!("rollout-2026-09-22T20-42-27-{OWNER}_{SEGMENT}.jsonl");
  let middle_path = write(root.path(), &name, &middle);
  let head = write(
    root.path(),
    "head.jsonl",
    &[
      meta(OWNER, 4, Some(base(SEGMENT, &prefix))),
      message(OWNER, 5, "current"),
    ],
  );
  let loaded = load(&source, &head);
  assert_eq!(loaded.reference.id, OWNER);
  assert_eq!(texts(&loaded), ["original", "retained", "current"]);
  assert_eq!(source.list_session_relations().unwrap().len(), 1);

  // A segment alias still requires exact bytes, ordinal, and filename owner.
  let mut wrong_cutoff = base(SEGMENT, &prefix);
  wrong_cutoff["end_byte_offset"] = json!(wrong_cutoff["end_byte_offset"].as_u64().unwrap() - 1);
  write(root.path(), "head.jsonl", &[meta(OWNER, 4, Some(wrong_cutoff))]);
  assert!(source.history_segments(&head).unwrap_err().contains("unavailable"));
  write(
    root.path(),
    "head.jsonl",
    &[meta(OWNER, 4, Some(base(SEGMENT, &prefix)))],
  );
  fs::rename(
    &middle_path,
    root
      .path()
      .join(format!("rollout-2026-09-22T20-42-27-{SEGMENT}_{SEGMENT}.jsonl")),
  )
  .unwrap();
  assert!(source.history_segments(&head).unwrap_err().contains("unavailable"));
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

fn append(path: &Path, bytes: &[u8]) {
  use std::io::Write;
  fs::OpenOptions::new()
    .append(true)
    .open(path)
    .unwrap()
    .write_all(bytes)
    .unwrap();
}

fn incremental_fixture(prefix_bytes: usize) -> (TempDir, CodexSessionSource, PathBuf) {
  let root = TempDir::new().unwrap();
  let source = CodexSessionSource::new(Some(root.path().into()));
  let prefix = vec![meta("thread", 0, None), message("thread", 1, &"x".repeat(prefix_bytes))];
  write(root.path(), "old.jsonl", &prefix);
  let head = write(
    root.path(),
    "head.jsonl",
    &[meta("thread", 2, Some(base("thread", &prefix)))],
  );
  (root, source, head)
}

#[test]
fn incremental_work_depends_on_appended_bytes_not_inherited_history() {
  for prefix_bytes in [64 * 1024, 1024 * 1024] {
    let (_root, source, head) = incremental_fixture(prefix_bytes);
    let mut reader = CodexHistoryReader::new(head.clone(), true, 32 * 1024 * 1024);
    assert!(reader.poll(&source).unwrap().unwrap().reset);
    let before = reader.stats();
    for _ in 0..10 {
      assert!(reader.poll(&source).unwrap().is_none());
    }
    assert_eq!(
      reader.stats(),
      before,
      "unchanged reads must do no parsing or discovery"
    );
    let mut appended_bytes = 0;
    for ordinal in 3..13 {
      let row = format!("{}\n", message("thread", ordinal, "incremental"));
      appended_bytes += row.len() as u64;
      append(&head, row.as_bytes());
      let update = reader.poll(&source).unwrap().unwrap();
      assert!(!update.reset);
      assert_eq!(update.records.len(), 1);
    }
    let after = reader.stats();
    assert_eq!(after.source_bytes_read - before.source_bytes_read, appended_bytes);
    assert_eq!(after.rows_parsed - before.rows_parsed, 10);
    assert_eq!(after.lineage_resolutions, before.lineage_resolutions);
    assert!(after.guard_bytes_read - before.guard_bytes_read < 50 * 1024);
  }
}

#[test]
fn incremental_reader_buffers_utf8_and_rebuilds_after_a_rejected_batch() {
  let (_root, source, head) = incremental_fixture(32);
  let mut reader = CodexHistoryReader::new(head.clone(), false, 1024 * 1024);
  reader.poll(&source).unwrap();
  let row = format!("{}\n", message("thread", 3, "中文"));
  let split = row.find('中').unwrap() + 1;
  append(&head, &row.as_bytes()[..split]);
  assert!(reader.poll(&source).unwrap().is_none());
  append(&head, &row.as_bytes()[split..]);
  let update = reader.poll(&source).unwrap().unwrap();
  assert!(!update.reset);
  assert_eq!(update.reference.message_count, 2);

  let good_prefix = fs::read(&head).unwrap();
  append(
    &head,
    format!("{}\nnot json\n", message("thread", 4, "must survive retry")).as_bytes(),
  );
  assert!(reader.poll(&source).is_err());
  assert!(reader.poll(&source).is_err());
  fs::write(&head, good_prefix).unwrap();
  append(
    &head,
    format!("{}\n", message("thread", 4, "must survive retry")).as_bytes(),
  );
  let update = reader.poll(&source).unwrap().unwrap();
  assert!(update.reset);
  assert_eq!(update.reference.message_count, 3);
  assert!(
    update
      .records
      .iter()
      .flat_map(|record| &record.events)
      .any(|event| { matches!(event, AgentEvent::Message(message) if message.text == "must survive retry") })
  );
}

#[test]
fn incremental_reader_preserves_normalizer_state_between_batches() {
  let (_root, source, head) = incremental_fixture(32);
  let mut reader = CodexHistoryReader::new(head.clone(), false, 1024 * 1024);
  reader.poll(&source).unwrap();
  append(
    &head,
    format!(
      "{}\n",
      json!({
        "type":"inter_agent_communication_metadata","ordinal":3,"payload":{"trigger_turn":true}
      })
    )
    .as_bytes(),
  );
  reader.poll(&source).unwrap();
  append(
    &head,
    format!(
      "{}\n",
      json!({
        "type":"response_item","ordinal":4,"payload":{
          "type":"agent_message","id":"message","author":"/root/worker","recipient":"/root",
          "content":[{"type":"input_text","text":"next task"}]
        }
      })
    )
    .as_bytes(),
  );
  let update = reader.poll(&source).unwrap().unwrap();
  assert!(!update.reset);
  assert!(update.records.iter().flat_map(|record| &record.events).any(|event| {
    matches!(event, AgentEvent::AgentActivity(activity)
      if activity.communication.as_ref().is_some_and(|communication| communication.trigger_turn == Some(true)))
  }));
}

#[test]
fn incremental_reader_resets_for_replacement_truncation_and_rewrite_with_growth() {
  let (root, source, head) = incremental_fixture(32);
  let original = fs::read(&head).unwrap();
  let mut reader = CodexHistoryReader::new(head.clone(), false, 1024 * 1024);
  reader.poll(&source).unwrap();
  let replacement = root.path().join("replacement");
  fs::write(&replacement, &original).unwrap();
  fs::rename(replacement, &head).unwrap();
  assert!(reader.poll(&source).unwrap().unwrap().reset);
  append(&head, format!("{}\n", message("thread", 3, "old message")).as_bytes());
  assert!(!reader.poll(&source).unwrap().unwrap().reset);
  fs::write(&head, &original).unwrap();
  assert!(reader.poll(&source).unwrap().unwrap().reset);

  // Changed ownership metadata at the beginning of a growing file must not
  // continue using the old normalizer, even though its inode was retained.
  let changed = String::from_utf8(original).unwrap().replace("/test", "/else");
  fs::write(&head, format!("{changed}{}\n", message("thread", 3, "new message"))).unwrap();
  let update = reader.poll(&source).unwrap().unwrap();
  assert!(update.reset);
  assert_eq!(update.reference.cwd.as_deref(), Some("/else"));
  assert_eq!(update.reference.message_count, 2);

  // A growing rewrite that keeps the owning metadata unchanged is detected
  // by the bounded guard immediately before the old byte cursor.
  let rewritten = fs::read_to_string(&head).unwrap().replace("new message", "edited text");
  fs::write(&head, format!("{rewritten}{}\n", message("thread", 4, "appended"))).unwrap();
  assert!(reader.poll(&source).unwrap().unwrap().reset);
}

#[test]
fn incremental_reader_revalidates_when_source_roots_change() {
  let (root, source, head) = incremental_fixture(32);
  let mut reader = CodexHistoryReader::new(head, false, 1024 * 1024);
  reader.poll(&source).unwrap();
  let restricted = root.path().join("restricted");
  fs::create_dir(&restricted).unwrap();
  assert!(reader.poll(&CodexSessionSource::new(Some(restricted))).is_err());
  assert!(reader.poll(&source).unwrap().unwrap().reset);
}

#[test]
fn parent_appends_outside_the_cutoff_do_not_reparse_the_child() {
  let (root, source, head) = incremental_fixture(32);
  let parent = root.path().join("old.jsonl");
  let mut reader = CodexHistoryReader::new(head, false, 1024 * 1024);
  reader.poll(&source).unwrap();
  let before = reader.stats();
  append(
    &parent,
    format!("{}\n", message("thread", 2, "parent continues")).as_bytes(),
  );
  assert!(reader.poll(&source).unwrap().is_none());
  let after = reader.stats();
  assert_eq!(after.source_bytes_read, before.source_bytes_read);
  assert_eq!(after.rows_parsed, before.rows_parsed);
  assert_eq!(after.lineage_resolutions, before.lineage_resolutions);
  assert!(after.guard_bytes_read > before.guard_bytes_read);
  assert!(reader.poll(&source).unwrap().is_none());
  assert_eq!(reader.stats(), after);

  // A same-inode rewrite within the protected prefix plus growth is not an
  // append. The saved bytes at the exclusive cutoff force a cold replacement.
  let edited = fs::read_to_string(&parent)
    .unwrap()
    .replace(&"x".repeat(32), &"y".repeat(32));
  fs::write(
    &parent,
    format!("{edited}{}\n", message("thread", 3, "later parent work")),
  )
  .unwrap();
  let update = reader.poll(&source).unwrap().unwrap();
  assert!(update.reset);
  assert_eq!(update.reference.message_count, 1);
  assert!(
    update
      .records
      .iter()
      .flat_map(|record| &record.events)
      .any(|event| { matches!(event, AgentEvent::Message(message) if message.text == "y".repeat(32)) })
  );
}

#[test]
fn fragmented_long_rows_are_parsed_only_after_their_newline_arrives() {
  let (_root, source, head) = incremental_fixture(32);
  let mut reader = CodexHistoryReader::new(head.clone(), false, 4 * 1024 * 1024);
  reader.poll(&source).unwrap();
  let before = reader.stats();
  let row = format!("{}\n", message("thread", 3, &"x".repeat(1024 * 1024)));
  let mut updates = 0;
  for chunk in row.as_bytes().chunks(4096) {
    append(&head, chunk);
    if let Some(update) = reader.poll(&source).unwrap() {
      assert!(!update.reset);
      assert_eq!(update.records.len(), 1);
      updates += 1;
    }
  }
  assert_eq!(updates, 1);
  assert_eq!(reader.stats().rows_parsed - before.rows_parsed, 1);
  assert_eq!(
    reader.stats().source_bytes_read - before.source_bytes_read,
    row.len() as u64
  );
}

#[test]
#[ignore = "manual comparison of full reloads and incremental appends at 1 and 10 MiB"]
fn benchmark_linked_history_appends() {
  use std::time::Instant;
  for mib in [1, 10] {
    let root = TempDir::new().unwrap();
    let source = CodexSessionSource::new(Some(root.path().into()));
    let mut prefix = vec![meta("thread", 0, None)];
    for ordinal in 1..=mib * 256 {
      prefix.push(message("thread", ordinal, &"x".repeat(4096)));
    }
    write(root.path(), "old.jsonl", &prefix);
    let head_ordinal = prefix.len() as u64;
    let head = write(
      root.path(),
      "head.jsonl",
      &[meta("thread", head_ordinal, Some(base("thread", &prefix)))],
    );
    let mut reader = CodexHistoryReader::new(head.clone(), false, 32 * 1024 * 1024);
    reader.poll(&source).unwrap();
    let before = reader.stats();
    let mut incremental = std::time::Duration::ZERO;
    let mut reload = std::time::Duration::ZERO;
    for ordinal in head_ordinal + 1..head_ordinal + 11 {
      append(&head, format!("{}\n", message("thread", ordinal, "next")).as_bytes());
      let started = Instant::now();
      reader.poll(&source).unwrap();
      incremental += started.elapsed();
      let started = Instant::now();
      source
        .load_session_records_path(&head, false, 32 * 1024 * 1024)
        .unwrap();
      reload += started.elapsed();
    }
    let after = reader.stats();
    println!(
      "{mib} MiB, 10 appends: reload={reload:?}, incremental={incremental:?}, body_bytes={}, rows={}, guard_bytes={}, lineage_resolutions={}",
      after.source_bytes_read - before.source_bytes_read,
      after.rows_parsed - before.rows_parsed,
      after.guard_bytes_read - before.guard_bytes_read,
      after.lineage_resolutions - before.lineage_resolutions
    );
  }
}
