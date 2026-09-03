use super::*;
use tempfile::TempDir;

struct Fixture {
  directory: TempDir,
  path: PathBuf,
  database: rusqlite::Connection,
}

impl Fixture {
  fn new() -> Self {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("opencode.db");
    let database = rusqlite::Connection::open(&path).unwrap();
    database.execute_batch(r#"
      pragma journal_mode = wal;
      create table session (id text primary key, parent_id text, directory text, title text, time_created integer, time_updated integer);
      create table message (id text primary key, session_id text, time_created integer, data text);
      create table part (id text primary key, message_id text, session_id text, time_created integer, data text);
      insert into session values ('one', null, '/tmp', null, 1, 1), ('other', null, '/tmp', null, 1, 1);
      insert into message values ('m1', 'one', 1, '{"role":"user"}');
      insert into part values ('p1', 'm1', 'one', 1, '{"type":"text","text":"hello"}');
    "#).unwrap();
    Self {
      directory,
      path,
      database,
    }
  }

  fn reader(&self, native: bool) -> SessionReader {
    let source = OpenCodeSessionSource::new(Some(self.path.clone()));
    let header = source
      .list_session_headers()
      .unwrap()
      .into_iter()
      .find(|h| h.id == "one")
      .unwrap();
    SessionReader::new(
      CatalogEntry {
        key: "one".into(),
        provider: Provider::OpenCode,
        header,
      },
      native,
      self.directory.path().into(),
    )
    .unwrap()
  }

  fn poll(&self, reader: &mut SessionReader) -> Result<bool, String> {
    // Force reconciliation, including the case where a platform's mtime
    // granularity coalesces test writes. No timing-dependent sleeps needed.
    reader.poll_database(versions(&self.path, true))
  }
}

#[test]
fn unrelated_writes_and_wal_checkpoint_do_not_publish_or_reset() {
  let fixture = Fixture::new();
  for native in [false, true] {
    let mut reader = fixture.reader(native);
    let initial = reader.snapshot.clone();
    assert!(!reader.poll().unwrap());
    fixture
      .database
      .execute_batch("update session set time_updated = time_updated + 1 where id = 'other'")
      .unwrap();
    assert!(!fixture.poll(&mut reader).unwrap());
    fixture
      .database
      .execute_batch("pragma wal_checkpoint(truncate)")
      .unwrap();
    assert!(!fixture.poll(&mut reader).unwrap());
    assert_eq!(reader.snapshot.generation, initial.generation);
    assert_eq!(reader.snapshot.revision, initial.revision);
    assert!(Arc::ptr_eq(&initial.records[1], &reader.snapshot.records[1]));
  }
}

#[test]
fn append_keeps_generation_and_reuses_history_despite_session_timestamp_change() {
  let fixture = Fixture::new();
  let mut reader = fixture.reader(false);
  let initial = reader.snapshot.clone();
  fixture
    .database
    .execute_batch(
      r#"
    insert into message values ('m2', 'one', 2, '{"role":"user"}');
    insert into part values ('p2', 'm2', 'one', 2, '{"type":"text","text":"world"}');
    update session set time_updated = 2 where id = 'one';
  "#,
    )
    .unwrap();
  assert!(fixture.poll(&mut reader).unwrap());
  assert_eq!(reader.snapshot.generation, initial.generation);
  assert_eq!(reader.snapshot.records.len(), 3);
  assert!(Arc::ptr_eq(&reader.snapshot.records[1], &initial.records[1]));
  assert_eq!(reader.snapshot.entry.header.timestamp.as_deref(), Some("1"));
  assert_eq!(reader.snapshot.entry.header.updated_at.as_deref(), Some("2"));
  assert!(!fixture.poll(&mut reader).unwrap());
}

#[test]
fn native_only_changes_are_visible_only_when_requested() {
  let fixture = Fixture::new();
  let mut plain = fixture.reader(false);
  let mut native = fixture.reader(true);
  let initial = native.snapshot.generation.clone();
  fixture
    .database
    .execute_batch(r#"update message set data = '{"role":"user","future_field":42}' where id = 'm1'"#)
    .unwrap();
  assert!(!fixture.poll(&mut plain).unwrap());
  assert!(fixture.poll(&mut native).unwrap());
  assert_ne!(native.snapshot.generation, initial);
  let initial = native.snapshot.generation.clone();
  fixture
    .database
    .execute_batch("update session set time_updated = 2 where id = 'one'")
    .unwrap();
  let plain_generation = plain.snapshot.generation.clone();
  assert!(fixture.poll(&mut plain).unwrap(), "metadata-only commit");
  assert_eq!(plain.snapshot.generation, plain_generation);
  assert!(fixture.poll(&mut native).unwrap());
  assert_ne!(native.snapshot.generation, initial);
}

#[test]
fn edits_deletions_and_reordering_reset_even_without_timestamp_changes() {
  let fixture = Fixture::new();
  let mut reader = fixture.reader(false);
  for sql in [
    // Same-length edit in this session plus a timestamp change elsewhere.
    r#"update part set data = '{"type":"text","text":"edits"}' where id = 'p1'; update session set time_updated = 2 where id = 'other';"#,
    r#"insert into message values ('m0', 'one', 0, '{"role":"user"}')"#,
    "delete from message where id = 'm1'",
  ] {
    let generation = reader.snapshot.generation.clone();
    fixture.database.execute_batch(sql).unwrap();
    assert!(fixture.poll(&mut reader).unwrap());
    assert_ne!(reader.snapshot.generation, generation);
    let fresh = fixture.reader(false);
    assert_eq!(
      serde_json::to_value(reader.snapshot.records.iter().map(|r| &r.record).collect::<Vec<_>>()).unwrap(),
      serde_json::to_value(fresh.snapshot.records.iter().map(|r| &r.record).collect::<Vec<_>>()).unwrap()
    );
  }
}

#[test]
fn malformed_update_preserves_last_good_snapshot_and_recovers() {
  let fixture = Fixture::new();
  let mut reader = fixture.reader(false);
  let initial = reader.snapshot.clone();
  fixture
    .database
    .execute_batch("update part set data = 'invalid'")
    .unwrap();
  assert!(fixture.poll(&mut reader).is_err());
  assert_eq!(reader.snapshot.generation, initial.generation);
  assert_eq!(reader.snapshot.revision, initial.revision);
  fixture
    .database
    .execute_batch(r#"update part set data = '{"type":"text","text":"hello"}'"#)
    .unwrap();
  assert!(!fixture.poll(&mut reader).unwrap());
}

#[cfg(unix)]
#[test]
fn replaced_database_resets_even_when_contents_match() {
  let fixture = Fixture::new();
  let mut reader = fixture.reader(false);
  let initial = reader.snapshot.generation.clone();
  fixture
    .database
    .execute_batch("pragma wal_checkpoint(truncate)")
    .unwrap();
  let replacement = fixture.directory.path().join("replacement.db");
  std::fs::copy(&fixture.path, &replacement).unwrap();
  drop(fixture.database);
  std::fs::rename(replacement, &fixture.path).unwrap();
  assert!(reader.poll().unwrap());
  assert_ne!(reader.snapshot.generation, initial);
}

#[test]
fn zcode_uses_its_identity_and_cached_sqlite_reconciliation() {
  let fixture = Fixture::new();
  let mut entry = fixture.reader(false).snapshot.entry;
  entry.provider = Provider::ZCode;
  let mut reader = SessionReader::new(entry, true, fixture.path.clone()).unwrap();
  assert!(
    reader
      .snapshot
      .records
      .iter()
      .all(|record| record.topic == "zcode.one" && record.session.provider == Provider::ZCode)
  );
  assert!(
    reader
      .snapshot
      .records
      .iter()
      .flat_map(|record| &record.record.events)
      .all(|event| serde_json::to_value(event).unwrap()["provider"] == "zcode")
  );
  let initial = reader.snapshot.generation.clone();
  fixture
    .database
    .execute_batch(r#"update part set data = '{"type":"text","text":"changed"}' where id = 'p1'"#)
    .unwrap();
  assert!(fixture.poll(&mut reader).unwrap());
  assert_ne!(reader.snapshot.generation, initial);
  assert!(!fixture.poll(&mut reader).unwrap());
}

#[test]
fn grouped_files_buffer_partial_rows_reset_edits_and_preserve_native() {
  use std::io::Write;
  use tokn_session_client::AgentClient;
  for provider in [Provider::WorkBuddy, Provider::Dsh] {
    let root = TempDir::new().unwrap();
    let (relative, contents, append) = match provider {
      Provider::WorkBuddy => (
        "projects/work/session.jsonl",
        "{\"type\":\"message\",\"id\":\"one\",\"sessionId\":\"session\",\"role\":\"user\",\"content\":\"hello\",\"timestamp\":1}\n",
        "{\"type\":\"future\",\"id\":\"one\",\"value\":42}",
      ),
      _ => (
        "session/session.jsonl",
        "{\"type\":\"session\",\"version\":0,\"id\":\"session\",\"createdAt\":1,\"delegationDepth\":0}\n{\"type\":\"future\",\"seq\":0,\"value\":\"hello\"}\n",
        "{\"type\":\"future\",\"seq\":1,\"value\":42}",
      ),
    };
    let path = root.path().join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, contents).unwrap();
    let header = AgentClient::list_session_headers(crate::providers::source(provider), Some(root.path().into()))
      .unwrap()
      .remove(0);
    let entry = CatalogEntry {
      key: "session".into(),
      provider,
      header,
    };
    let mut reader = SessionReader::new(entry, true, root.path().into()).unwrap();
    let initial = reader.snapshot.clone();
    assert!(initial.records.iter().any(|r| r.record.native.is_some()));
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(append.as_bytes()).unwrap();
    assert!(
      !reader.poll_grouped_file(versions(&path, false)).unwrap(),
      "no complete new row"
    );
    file.write_all(b"\n").unwrap();
    assert!(reader.poll_grouped_file(versions(&path, false)).unwrap());
    assert_eq!(initial.generation, reader.snapshot.generation);
    assert_eq!(initial.records.len() + 1, reader.snapshot.records.len());
    let ids: std::collections::HashSet<_> = reader.snapshot.records.iter().map(|r| &r.record.record_id).collect();
    assert_eq!(
      ids.len(),
      reader.snapshot.records.len(),
      "duplicate native IDs must not collapse"
    );
    std::fs::write(&path, format!("{}{append}\n", contents.replace("hello", "edits"))).unwrap();
    assert!(reader.poll_grouped_file(versions(&path, false)).unwrap());
    assert_ne!(initial.generation, reader.snapshot.generation);
    let last_good = reader.snapshot.revision;
    std::fs::write(&path, "invalid\n").unwrap();
    assert!(reader.poll_grouped_file(versions(&path, false)).is_err());
    assert_eq!(reader.snapshot.revision, last_good);
  }
}

#[test]
fn assembled_dsh_output_resets_prior_stream_batches() {
  use tokn_session_client::{AgentClient, Source};
  let root = TempDir::new().unwrap();
  let path = root.path().join("session.jsonl");
  let fixture = include_str!("../../../dsh/fixtures/basic/session.jsonl");
  let split = fixture.find("{\"type\":\"assistant/message\"").unwrap();
  std::fs::write(&path, &fixture[..split]).unwrap();
  let header = AgentClient::list_session_headers(Source::Dsh, Some(root.path().into()))
    .unwrap()
    .remove(0);
  let entry = CatalogEntry {
    key: "dsh".into(),
    provider: Provider::Dsh,
    header,
  };
  let mut reader = SessionReader::new(entry, false, root.path().into()).unwrap();
  let initial = reader.snapshot.generation.clone();
  std::fs::write(&path, fixture).unwrap();
  assert!(reader.poll_grouped_file(versions(&path, false)).unwrap());
  assert_ne!(reader.snapshot.generation, initial);
  let history = AgentClient::load_session(Source::Dsh, Some(root.path().into()), "dsh-fixture").unwrap();
  let events: Vec<_> = reader
    .snapshot
    .records
    .iter()
    .flat_map(|record| &record.record.events)
    .collect();
  assert_eq!(
    serde_json::to_value(events).unwrap(),
    serde_json::to_value(history.events).unwrap()
  );
}

#[test]
fn workbuddy_catalog_wal_updates_followed_presentation() {
  use tokn_session_client::{AgentClient, Source};
  let root = TempDir::new().unwrap();
  let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../workbuddy/fixtures");
  let path = root.path().join("projects/fixture-workspace/wb-shell-command.jsonl");
  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::copy(
    fixtures.join("projects/fixture-workspace/wb-shell-command.jsonl"),
    &path,
  )
  .unwrap();
  std::fs::copy(fixtures.join("workbuddy.db"), root.path().join("workbuddy.db")).unwrap();
  let header = AgentClient::list_session_headers(Source::WorkBuddy, Some(root.path().into()))
    .unwrap()
    .into_iter()
    .find(|h| h.id == "wb-shell-command")
    .unwrap();
  let entry = CatalogEntry {
    key: "wb".into(),
    provider: Provider::WorkBuddy,
    header,
  };
  let mut reader = SessionReader::new(entry, false, root.path().into()).unwrap();
  let initial = reader.snapshot.generation.clone();
  let database = rusqlite::Connection::open(root.path().join("workbuddy.db")).unwrap();
  database.execute_batch("pragma journal_mode=wal; update sessions set title = 'Changed catalog title', custom_title = 'Changed catalog title' where id = 'wb-shell-command';").unwrap();
  assert!(reader.poll().unwrap());
  assert_eq!(
    reader.snapshot.entry.header.title.as_deref(),
    Some("Changed catalog title")
  );
  assert_eq!(reader.snapshot.generation, initial);
}
