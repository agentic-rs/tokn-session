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
