use super::*;
use tempfile::TempDir;

struct Fixture {
  _directory: TempDir,
  database: Connection,
  source: OpenCodeSessionSource,
}

impl Fixture {
  fn new() -> Self {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("opencode.db");
    let database = Connection::open(&path).unwrap();
    database.execute_batch(r#"
      pragma journal_mode = wal;
      create table session (id text primary key, parent_id text, directory text, time_created integer, time_updated integer);
      create table message (id text primary key, session_id text, time_created integer, data text);
      create table part (id text primary key, message_id text, session_id text, time_created integer, data text);
      create table session_entry (id text primary key, session_id text, type text, time_created integer, data text);
      insert into session values ('one', null, '/tmp', 1, 1), ('other', null, '/tmp', 1, 1);
      insert into message values ('m1', 'one', 1, '{"role":"user","model":{"providerID":"p","modelID":"a"}}');
      insert into message values ('m2', 'one', 2, '{"role":"user","model":{"providerID":"p","modelID":"a"}}');
      insert into message values ('m3', 'one', 3, '{"role":"user","model":{"providerID":"p","modelID":"a"}}');
      insert into part values ('p1', 'm1', 'one', 1, '{"type":"text","text":"hello"}');
      insert into part values ('p2', 'm2', 'one', 2, '{"type":"text","text":"world"}');
    "#).unwrap();
    Self {
      _directory: directory,
      database,
      source: OpenCodeSessionSource::new(Some(path)),
    }
  }

  fn load(&self, cache: &mut OpenCodeSessionCache, native: bool) -> CachedSessionRecords {
    let cached = self
      .source
      .load_session_records_cached_exact("one", native, cache)
      .unwrap();
    // The independent historical reader is the normalization/order oracle.
    let full = self.source.load_session_records_exact("one", native).unwrap();
    assert_eq!(
      serde_json::to_value(&cached.reference).unwrap(),
      serde_json::to_value(&full.reference).unwrap()
    );
    assert_eq!(
      serde_json::to_value(cached.records.iter().map(AsRef::as_ref).collect::<Vec<_>>()).unwrap(),
      serde_json::to_value(full.records).unwrap()
    );
    cached
  }
}

#[test]
fn unchanged_snapshots_reuse_decoding_normalization_and_records() {
  let fixture = Fixture::new();
  for native in [false, true] {
    let mut cache = OpenCodeSessionCache::default();
    let first = fixture.load(&mut cache, native);
    assert_eq!((cache.decoded, cache.normalized), (4, 4));
    fixture
      .database
      .execute_batch("update session set time_updated = time_updated + 1 where id = 'other';")
      .unwrap();
    let next = fixture.load(&mut cache, native);
    assert_eq!((cache.decoded, cache.normalized), (4, 4));
    assert!(first.records.iter().zip(next.records).all(|(a, b)| Arc::ptr_eq(a, &b)));
    fixture
      .database
      .execute_batch("pragma wal_checkpoint(truncate);")
      .unwrap();
    fixture.load(&mut cache, native);
    assert_eq!((cache.decoded, cache.normalized), (4, 4));
  }
}

#[test]
fn append_and_timestamp_free_edits_only_decode_changed_records() {
  let fixture = Fixture::new();
  let mut cache = OpenCodeSessionCache::default();
  let first = fixture.load(&mut cache, true);
  fixture
    .database
    .execute_batch(
      r#"
    update session set time_updated = 2 where id = 'one';
    insert into message values ('m4', 'one', 4, '{"role":"user"}');
  "#,
    )
    .unwrap();
  let appended = fixture.load(&mut cache, true);
  assert_eq!((cache.decoded, cache.normalized), (6, 6));
  assert!(Arc::ptr_eq(&first.records[1], &appended.records[1]));
  fixture
    .database
    .execute_batch(r#"update part set data = '{"type":"text","text":"edits"}' where id = 'p1';"#)
    .unwrap();
  let edited = fixture.load(&mut cache, true);
  assert_eq!((cache.decoded, cache.normalized), (7, 7));
  assert!(!Arc::ptr_eq(&appended.records[1], &edited.records[1]));
  assert!(Arc::ptr_eq(&appended.records[2], &edited.records[2]));
}

#[test]
fn model_changes_recompute_dependent_records_until_state_converges() {
  let fixture = Fixture::new();
  let mut cache = OpenCodeSessionCache::default();
  let first = fixture.load(&mut cache, false);
  fixture
    .database
    .execute_batch(
      r#"update message set data = '{"role":"user","model":{"providerID":"p","modelID":"b"}}' where id = 'm1';"#,
    )
    .unwrap();
  let changed = fixture.load(&mut cache, false);
  assert_eq!((cache.decoded, cache.normalized), (5, 6));
  assert!(!Arc::ptr_eq(&first.records[2], &changed.records[2]));
  assert!(Arc::ptr_eq(&first.records[3], &changed.records[3]));
}

#[test]
fn deletion_reordering_and_cross_session_parts_match_full_history() {
  let fixture = Fixture::new();
  let mut cache = OpenCodeSessionCache::default();
  fixture.load(&mut cache, true);
  for sql in [
    "delete from part where id = 'p1'",
    "update message set time_created = 0 where id = 'm3'",
    "update part set session_id = 'other' where id = 'p2'",
    "delete from message where id = 'm1'",
  ] {
    fixture.database.execute_batch(sql).unwrap();
    fixture.load(&mut cache, true);
  }
  assert!(!cache.rows.contains_key("message:m1"));
}

#[test]
fn failures_do_not_install_partial_cache_and_limits_are_enforced() {
  let fixture = Fixture::new();
  let mut cache = OpenCodeSessionCache::default();
  let first = fixture.load(&mut cache, false);
  fixture
    .database
    .execute_batch("update part set data = 'broken' where id = 'p2'")
    .unwrap();
  assert!(
    fixture
      .source
      .load_session_records_cached_exact("one", false, &mut cache)
      .is_err()
  );
  assert_eq!((cache.decoded, cache.normalized), (4, 4));
  assert!(Arc::ptr_eq(&first.records[1], &cache.rows["message:m1"].record));
  fixture
    .database
    .execute_batch(r#"update part set data = '{"type":"text","text":"world"}' where id = 'p2'"#)
    .unwrap();
  fixture.load(&mut cache, false);
  assert_eq!((cache.decoded, cache.normalized), (4, 4));
  cache.max_source_bytes = Some(1);
  assert!(
    fixture
      .source
      .load_session_records_cached_exact("one", false, &mut cache)
      .err()
      .unwrap()
      .contains("size limit")
  );
  assert_eq!((cache.decoded, cache.normalized), (4, 4));
}

#[test]
fn cache_identity_includes_native_session_path_and_provider() {
  let mut fixture = Fixture::new();
  let mut cache = OpenCodeSessionCache::default();
  fixture.load(&mut cache, false);
  let native = fixture.load(&mut cache, true);
  assert!(native.records.iter().all(|r| r.native.is_some()));
  assert_eq!(cache.decoded, 8);
  fixture
    .source
    .load_session_records_cached_exact("other", true, &mut cache)
    .unwrap();
  fixture.load(&mut cache, true);
  assert_eq!(cache.decoded, 13);
  let other = Fixture::new();
  other.load(&mut cache, true);
  assert_eq!(cache.decoded, 17);
  fixture.source.flavor = SessionDatabaseFlavor::ZCode;
  fixture
    .database
    .execute_batch(r#"insert into session_entry values ('entry', 'one', 'note', 2, '{"text":"note"}')"#)
    .unwrap();
  fixture.load(&mut cache, true);
  assert_eq!(cache.decoded, 22);
  // Same path/session, different provider must also invalidate cached events.
  fixture.source.flavor = SessionDatabaseFlavor::OpenCode;
  fixture.load(&mut cache, true);
  assert_eq!(cache.decoded, 26);
}
