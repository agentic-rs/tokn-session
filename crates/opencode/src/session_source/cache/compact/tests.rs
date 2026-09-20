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

  fn load(
    &self,
    cache: &mut OpenCodeCompactCache,
    previous: &[Arc<NormalizedRecord>],
    native: bool,
  ) -> Vec<Arc<NormalizedRecord>> {
    let compact = self
      .source
      .load_session_records_compact_exact("one", native, cache)
      .unwrap();
    let records: Vec<_> = compact
      .records
      .into_iter()
      .map(|record| match record {
        CompactRecord::Reused(position) => previous[position].clone(),
        CompactRecord::Changed(record) => record,
      })
      .collect();
    let mut full = self.source.load_session_records_exact("one", native).unwrap();
    full.reference.preview = full.reference.preview.map(bound_preview);
    assert_eq!(
      serde_json::to_value(&compact.reference).unwrap(),
      serde_json::to_value(full.reference).unwrap()
    );
    assert_eq!(
      serde_json::to_value(records.iter().map(AsRef::as_ref).collect::<Vec<_>>()).unwrap(),
      serde_json::to_value(full.records).unwrap()
    );
    records
  }
}

#[test]
fn unchanged_and_tail_updates_reuse_checkpoints_without_decoding_history() {
  let fixture = Fixture::new();
  for native in [false, true] {
    let mut cache = OpenCodeCompactCache::default();
    let first = fixture.load(&mut cache, &[], native);
    assert_eq!((cache.decoded, cache.normalized), (4, 4));
    fixture
      .database
      .execute_batch(
        "update session set time_updated = time_updated + 1 where id = 'other'; pragma wal_checkpoint(truncate);",
      )
      .unwrap();
    let next = fixture.load(&mut cache, &first, native);
    assert_eq!((cache.decoded, cache.normalized), (4, 4));
    assert!(first.iter().zip(&next).all(|(a, b)| Arc::ptr_eq(a, b)));
    fixture
      .database
      .execute_batch("insert into message values ('m4', 'one', 4, '{\"role\":\"user\"}')")
      .unwrap();
    let appended = fixture.load(&mut cache, &next, native);
    assert_eq!((cache.decoded, cache.normalized), (5, 5));
    assert!(next.iter().zip(&appended).all(|(a, b)| Arc::ptr_eq(a, b)));
    fixture
      .database
      .execute_batch("delete from message where id='m4'")
      .unwrap();
  }
}

#[test]
fn edits_state_changes_deletion_and_reordering_match_full_normalization() {
  let fixture = Fixture::new();
  let mut cache = OpenCodeCompactCache::default();
  let mut records = fixture.load(&mut cache, &[], true);
  fixture
    .database
    .execute_batch(
      "update message set data='{\"role\":\"user\",\"model\":{\"providerID\":\"p\",\"modelID\":\"b\"}}' where id='m1'",
    )
    .unwrap();
  records = fixture.load(&mut cache, &records, true);
  assert_eq!(
    (cache.decoded, cache.normalized),
    (6, 6),
    "changed input and dependent next row until state converges"
  );
  for sql in [
    "update part set data='{\"type\":\"text\",\"text\":\"edits\"}' where id='p1'",
    "delete from message where id='m1'",
    "update message set time_created=0 where id='m3'",
    "delete from part where id='p2'",
  ] {
    fixture.database.execute_batch(sql).unwrap();
    records = fixture.load(&mut cache, &records, true);
  }
  fixture
    .database
    .execute_batch("update message set data='broken' where id='m2'")
    .unwrap();
  assert!(
    fixture
      .source
      .load_session_records_compact_exact("one", true, &mut cache)
      .is_err()
  );
  fixture
    .database
    .execute_batch("update message set data='{\"role\":\"user\"}' where id='m2'")
    .unwrap();
  fixture.load(&mut cache, &records, true);
}

#[test]
fn checkpoints_hold_no_record_bodies_and_share_unchanged_normalizer_state() {
  let fixture = Fixture::new();
  fixture
    .database
    .execute(
      "update part set data=? where id='p2'",
      [serde_json::json!({"type":"text", "text":"x".repeat(1024*1024)}).to_string()],
    )
    .unwrap();
  let mut cache = OpenCodeCompactCache::default();
  let records = fixture.load(&mut cache, &[], true);
  let weak: Vec<_> = records.iter().map(Arc::downgrade).collect();
  drop(records);
  assert!(
    weak.iter().all(|record| record.upgrade().is_none()),
    "cache must not retain normalized payloads"
  );
  assert_eq!(
    cache.preview.as_ref().unwrap().1,
    "hello",
    "only the first catalog preview survives"
  );
  assert!(Arc::ptr_eq(
    &cache.rows["message:m2"].before,
    &cache.rows["message:m2"].after
  ));
  assert!(Arc::ptr_eq(
    &cache.rows["message:m2"].after,
    &cache.rows["message:m3"].after
  ));
  cache.invalidate();
  assert!(cache.rows.is_empty());
  assert!(cache.preview.is_none());
}

#[test]
fn first_prompt_preview_is_bounded_without_truncating_normalized_content() {
  let fixture = Fixture::new();
  let prompt = "🙂".repeat(2000);
  fixture
    .database
    .execute(
      "update part set data=? where id='p1'",
      [serde_json::json!({"type":"text", "text":prompt}).to_string()],
    )
    .unwrap();
  let mut cache = OpenCodeCompactCache::default();
  let records = fixture.load(&mut cache, &[], true);
  assert_eq!(cache.preview.as_ref().unwrap().1.chars().count(), 512);
  assert!(
    records
      .iter()
      .flat_map(|record| &record.events)
      .any(|event| matches!(event,
    tokn_session_core::AgentEvent::Message(message) if message.text == prompt))
  );
  fixture.load(&mut cache, &records, true);
  assert_eq!((cache.decoded, cache.normalized), (4, 4));
}
