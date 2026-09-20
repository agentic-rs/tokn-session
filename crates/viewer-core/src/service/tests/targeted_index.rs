use super::*;
use std::io::Write;

fn append(path: &Path) {
  writeln!(
    std::fs::OpenOptions::new().append(true).open(path).unwrap(),
    "next record"
  )
  .unwrap();
}

fn fixture(index: Arc<SessionIndex>, path: &Path) -> (ViewerService, Arc<IndexingRepository>) {
  std::fs::write(path, "initial record\n").unwrap();
  let repository = indexing_repository(vec![IndexedLoadSpec {
    header: indexed_header(path.to_path_buf(), "active", None),
    messages: vec![indexed_message(Role::User, MessageDelivery::Unspecified)],
  }]);
  let service = ViewerService::new_with_index(repository.clone(), index);
  service.refresh_session_catalog().unwrap();
  service.refresh_pending_session_index().unwrap();
  (service, repository)
}

fn unrelated_source(provider: &str, id: &str) -> SourceReplacement {
  let key = SourceKey::new(provider, format!("fixture-{id}"));
  SourceReplacement::baseline(
    SourceState::new(key.clone(), "fixture", 0),
    vec![SessionMetadata::new(
      IndexedSessionKey::new(provider, key.source_key, id),
      format!("/fixture/{id}.jsonl"),
    )],
  )
}

#[test]
fn duplicate_hints_skip_header_reads_and_preserve_pending_body_checkpoint() {
  for provider in [ViewerProvider::Codex, ViewerProvider::Pi] {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("active.jsonl");
    std::fs::write(&path, "initial record\n").unwrap();
    let repository = indexing_repository_for(
      provider,
      vec![IndexedLoadSpec {
        header: indexed_header(path.clone(), "active", None),
        messages: vec![],
      }],
    );
    let index = Arc::new(SessionIndex::open_in_memory().unwrap());
    let service = ViewerService::new_with_index(repository.clone(), index.clone());
    service.refresh_session_catalog().unwrap();
    let paths = BTreeSet::from([path.clone()]);
    let source_key = index_source_key_for_path(provider, &path).unwrap();
    let before = index.source_state(&source_key).unwrap();
    for _ in 0..3 {
      let result = service.refresh_changed_file_provider_catalog(provider, &paths).unwrap();
      assert!(!result.changed);
      assert!(!result.retry_catalog_soon);
    }
    assert_eq!(repository.targeted_header_calls.load(Ordering::SeqCst), 0);
    assert_eq!(index.source_state(&source_key).unwrap(), before);

    append(&path);
    assert!(
      service
        .refresh_changed_file_provider_catalog(provider, &paths)
        .unwrap()
        .changed
    );
    let staged = index.source_state(&source_key).unwrap();
    assert!(pending_body_raw_cursor(&staged.as_ref().unwrap().cursor).is_some());
    assert!(
      !service
        .refresh_changed_file_provider_catalog(provider, &paths)
        .unwrap()
        .changed
    );
    assert_eq!(repository.targeted_header_calls.load(Ordering::SeqCst), 1);
    assert_eq!(index.source_state(&source_key).unwrap(), staged);
  }
}

#[test]
fn targeted_append_does_not_decode_unrelated_catalog_rows() {
  let directory = tempfile::tempdir().unwrap();
  let database = directory.path().join("index.sqlite");
  let index = Arc::new(SessionIndex::open(&database).unwrap());
  let path = directory.path().join("active.jsonl");
  let (service, repository) = fixture(index.clone(), &path);
  index
    .replace_sources(&[
      unrelated_source("codex", "same-provider-unrelated"),
      unrelated_source("pi", "other-provider-unrelated"),
    ])
    .unwrap();
  let connection = rusqlite::Connection::open(&database).unwrap();
  connection
    .execute(
      "UPDATE sessions SET title = x'ff' WHERE session_id LIKE '%unrelated'",
      [],
    )
    .unwrap();
  // SQLite accepts a blob in a TEXT column, but decoding that row into our
  // typed model fails. This makes an accidental whole-catalog read observable
  // without production counters or timing-sensitive assertions.
  assert!(index.list_all_sessions().is_err());
  append(&path);
  let refresh = service
    .refresh_changed_file_catalogs(BTreeMap::from([(ViewerProvider::Codex, BTreeSet::from([path]))]))
    .unwrap();
  assert!(refresh.changed);
  assert!(!refresh.retry_catalog_soon);
  assert_eq!(repository.targeted_header_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn changed_identity_or_duplicate_source_requests_full_catalog_without_replacement() {
  for duplicate in [false, true] {
    let directory = tempfile::tempdir().unwrap();
    let index = Arc::new(SessionIndex::open_in_memory().unwrap());
    let path = directory.path().join("active.jsonl");
    let (service, repository) = fixture(index.clone(), &path);
    let source_key = index_source_key_for_path(ViewerProvider::Codex, &path).unwrap();
    let before = index.source_state(&source_key).unwrap();
    if duplicate {
      index.replace_source(unrelated_source("codex", "active")).unwrap();
    } else {
      repository.targeted_headers.lock().unwrap().insert(
        (ViewerProvider::Codex, path.clone()),
        Ok(indexed_header(path.clone(), "replacement", None)),
      );
    }
    append(&path);
    let refresh = service
      .refresh_changed_file_provider_catalog(ViewerProvider::Codex, &BTreeSet::from([path]))
      .unwrap();
    assert!(!refresh.changed);
    assert!(refresh.retry_catalog_soon);
    assert_eq!(index.source_state(&source_key).unwrap(), before);
  }
}

#[test]
fn provider_catalog_does_not_decode_other_providers() {
  let directory = tempfile::tempdir().unwrap();
  let database = directory.path().join("index.sqlite");
  let index = Arc::new(SessionIndex::open(&database).unwrap());
  let path = directory.path().join("active.jsonl");
  let (service, _) = fixture(index.clone(), &path);
  rusqlite::Connection::open(&database)
    .unwrap()
    .execute("UPDATE sessions SET title = x'ff' WHERE session_id = 'active'", [])
    .unwrap();
  assert!(index.list_all_sessions().is_err());
  let snapshot = service.catalog_index_snapshot(ViewerProvider::OpenCode).unwrap();
  assert!(snapshot.existing_sessions.is_empty());
}

/// Run with `cargo test -p tokn-viewer-core targeted_catalog_benchmark -- --ignored --nocapture`.
/// Fixture construction is excluded from timings. Both versions exercise the
/// same existing targeted-refresh entry point, so this file can be copied to
/// an older checkout when measuring the change.
#[test]
#[ignore = "manual CPU regression measurement"]
fn targeted_catalog_benchmark() {
  let directory = tempfile::tempdir().unwrap();
  let index = Arc::new(SessionIndex::open_in_memory().unwrap());
  let path = directory.path().join("active.jsonl");
  let (service, repository) = fixture(index.clone(), &path);
  let unrelated = (0..5_000)
    .map(|number| unrelated_source("codex", &format!("other-{number}")))
    .collect::<Vec<_>>();
  index.replace_sources(&unrelated).unwrap();
  let paths = BTreeSet::from([path.clone()]);
  let start = std::time::Instant::now();
  for _ in 0..200 {
    service
      .refresh_changed_file_provider_catalog(ViewerProvider::Codex, &paths)
      .unwrap();
  }
  let duplicate_time = start.elapsed();
  let start = std::time::Instant::now();
  for _ in 0..100 {
    append(&path);
    service
      .refresh_changed_file_provider_catalog(ViewerProvider::Codex, &paths)
      .unwrap();
  }
  eprintln!(
    "targeted catalog: 5001 rows, 200 duplicate hints {:?}, 100 appends {:?}, header reads {}",
    duplicate_time,
    start.elapsed(),
    repository.targeted_header_calls.load(Ordering::SeqCst),
  );
}
