use std::{fs::OpenOptions, io::Write, path::Path, time::Duration};

use tempfile::TempDir;
use tokio::{io::AsyncWriteExt, net::TcpListener};
use tokn_session_core::{AgentEvent, Provider};
use tokn_session_relay::{
  ProviderRoot, RelayConfig,
  service_client::{RelaySubscription, SessionSnapshot, load_catalog},
  service_protocol::*,
  service_server::serve_listener,
};

const HEADER: &str = "{\"type\":\"session\",\"id\":\"pi-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n";
const MESSAGE: &str = "{\"type\":\"message\",\"id\":\"one\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n";

struct Server {
  endpoint: String,
  task: tokio::task::JoinHandle<Result<(), String>>,
}
impl Drop for Server {
  fn drop(&mut self) {
    self.task.abort();
  }
}

async fn server(root: &Path, provider: Provider, native: bool) -> Server {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let endpoint = format!("tcp://{}", listener.local_addr().unwrap());
  let mut config = RelayConfig::new(vec![ProviderRoot::new(provider, root.into())]);
  config.include_native = native;
  config.poll_interval = Duration::from_millis(20);
  Server {
    endpoint,
    task: tokio::spawn(serve_listener(listener, config)),
  }
}

async fn snapshot(subscription: &mut RelaySubscription) -> SessionSnapshot {
  tokio::time::timeout(Duration::from_secs(4), subscription.next_snapshot())
    .await
    .unwrap()
    .unwrap()
}

fn append(path: &Path, text: &str) {
  OpenOptions::new()
    .append(true)
    .open(path)
    .unwrap()
    .write_all(text.as_bytes())
    .unwrap();
}

fn messages(snapshot: &SessionSnapshot) -> Vec<&str> {
  snapshot
    .loaded
    .events
    .iter()
    .filter_map(|event| match event {
      AgentEvent::Message(message) => Some(message.text.as_str()),
      _ => None,
    })
    .collect()
}

#[tokio::test]
async fn subscribers_share_snapshot_generation_and_follow_without_duplicate_history() {
  let root = TempDir::new().unwrap();
  let path = root.path().join("session.jsonl");
  std::fs::write(&path, format!("{HEADER}{MESSAGE}")).unwrap();
  let server = server(root.path(), Provider::Pi, true).await;
  let catalog = load_catalog(&server.endpoint).await.unwrap();
  assert_eq!(catalog.entries.len(), 1);
  assert!(catalog.native);
  let key = &catalog.entries[0].key;
  let mut first = RelaySubscription::connect(&server.endpoint, key).await.unwrap();
  let initial = snapshot(&mut first).await;
  assert_eq!(messages(&initial), ["hello"]);
  assert!(initial.native.iter().any(Option::is_some));
  let mut second = RelaySubscription::connect(&server.endpoint, key).await.unwrap();
  let other = snapshot(&mut second).await;
  assert_eq!(
    initial.generation, other.generation,
    "one reader serves both subscribers"
  );
  append(&path, &MESSAGE.replace("one", "two").replace("hello", "world"));
  let update = snapshot(&mut first).await;
  assert_eq!(messages(&update), ["hello", "world"]);
  assert_eq!(update.generation, initial.generation);
  assert_ne!(update.revision, initial.revision);
  assert_eq!(messages(&snapshot(&mut second).await), ["hello", "world"]);
  // A reconnect receives a complete image, not only publications since connect.
  let mut reconnect = RelaySubscription::connect(&server.endpoint, key).await.unwrap();
  assert_eq!(messages(&snapshot(&mut reconnect).await), ["hello", "world"]);
  server.task.abort();
  assert!(
    tokio::time::timeout(Duration::from_secs(2), reconnect.next_snapshot())
      .await
      .unwrap()
      .is_err()
  );
}

#[tokio::test]
async fn partial_records_wait_for_newline_and_replacement_starts_a_generation() {
  let root = TempDir::new().unwrap();
  let path = root.path().join("session.jsonl");
  std::fs::write(&path, HEADER).unwrap();
  let server = server(root.path(), Provider::Pi, false).await;
  let key = load_catalog(&server.endpoint).await.unwrap().entries.remove(0).key;
  let mut subscription = RelaySubscription::connect(&server.endpoint, &key).await.unwrap();
  let initial = snapshot(&mut subscription).await;
  append(&path, MESSAGE.trim_end());
  tokio::time::sleep(Duration::from_millis(70)).await;
  append(&path, "\n");
  let next = snapshot(&mut subscription).await;
  assert_eq!(messages(&next), ["hello"]);
  assert_eq!(next.generation, initial.generation);
  assert!(next.native.iter().all(Option::is_none));
  let replacement = root.path().join("replacement");
  std::fs::write(&replacement, format!("{HEADER}{}", MESSAGE.replace("hello", "reset"))).unwrap();
  std::fs::rename(replacement, &path).unwrap();
  let reset = snapshot(&mut subscription).await;
  assert_ne!(next.generation, reset.generation);
  assert_eq!(messages(&reset), ["reset"]);
  std::fs::write(&path, HEADER).unwrap();
  let truncated = snapshot(&mut subscription).await;
  assert_ne!(reset.generation, truncated.generation);
  assert!(messages(&truncated).is_empty());
}

#[tokio::test]
async fn catalog_does_not_decode_invalid_bodies_and_follow_rejects_them() {
  let root = TempDir::new().unwrap();
  std::fs::write(root.path().join("session.jsonl"), format!("{HEADER}invalid-json\n")).unwrap();
  let server = server(root.path(), Provider::Pi, false).await;
  let catalog = load_catalog(&server.endpoint).await.unwrap();
  assert_eq!(catalog.entries.len(), 1);
  let mut subscription = RelaySubscription::connect(&server.endpoint, &catalog.entries[0].key)
    .await
    .unwrap();
  assert!(subscription.next_snapshot().await.is_err());
  let mut unknown = RelaySubscription::connect(&server.endpoint, "/tmp/arbitrary-file")
    .await
    .unwrap();
  assert!(
    unknown
      .next_snapshot()
      .await
      .unwrap_err()
      .contains("Unknown Relay session")
  );
}

#[tokio::test]
async fn opencode_changes_and_deletions_replace_the_snapshot() {
  let root = TempDir::new().unwrap();
  let path = root.path().join("opencode.db");
  let db = rusqlite::Connection::open(&path).unwrap();
  db.execute_batch(r#"
    pragma journal_mode = wal;
    create table session (id text primary key, parent_id text, directory text, time_created integer, time_updated integer);
    create table message (id text primary key, session_id text, time_created integer, data text);
    create table part (id text primary key, message_id text, session_id text, time_created integer, data text);
    insert into session values ('ses_1', null, '/tmp', 1, 1);
    insert into message values ('msg_1', 'ses_1', 1, '{"role":"user"}');
    insert into part values ('part_1', 'msg_1', 'ses_1', 1, '{"type":"text","text":"hello"}');
  "#).unwrap();
  let server = server(&path, Provider::OpenCode, true).await;
  let key = load_catalog(&server.endpoint).await.unwrap().entries.remove(0).key;
  let mut subscription = RelaySubscription::connect(&server.endpoint, &key).await.unwrap();
  let initial = snapshot(&mut subscription).await;
  assert_eq!(messages(&initial), ["hello"]);
  db.execute("update part set data = ?1", [r#"{"type":"text","text":"edited"}"#])
    .unwrap();
  let edited = snapshot(&mut subscription).await;
  assert_eq!(messages(&edited), ["edited"]);
  assert_ne!(initial.generation, edited.generation);
  db.execute_batch("delete from part; delete from message;").unwrap();
  assert!(messages(&snapshot(&mut subscription).await).is_empty());
}

#[tokio::test]
async fn opencode_subscribers_append_and_receive_metadata_without_replacing_history() {
  for native in [false, true] {
    let root = TempDir::new().unwrap();
    let path = root.path().join("opencode.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(r#"
      pragma journal_mode = wal;
      create table session (id text primary key, parent_id text, directory text, title text, time_created integer, time_updated integer);
      create table message (id text primary key, session_id text, time_created integer, data text);
      create table part (id text primary key, message_id text, session_id text, time_created integer, data text);
      insert into session values ('ses_1', null, '/tmp', null, 1, 1);
      insert into message values ('msg_1', 'ses_1', 1, '{"role":"user"}');
      insert into part values ('part_1', 'msg_1', 'ses_1', 1, '{"type":"text","text":"hello"}');
    "#).unwrap();
    let server = server(&path, Provider::OpenCode, native).await;
    let key = load_catalog(&server.endpoint).await.unwrap().entries.remove(0).key;
    let mut first = RelaySubscription::connect(&server.endpoint, &key).await.unwrap();
    let initial = snapshot(&mut first).await;
    let mut second = RelaySubscription::connect(&server.endpoint, &key).await.unwrap();
    assert_eq!(snapshot(&mut second).await.generation, initial.generation);
    db.execute_batch(
      r#"
      begin;
      insert into message values ('msg_2', 'ses_1', 2, '{"role":"user"}');
      insert into part values ('part_2', 'msg_2', 'ses_1', 2, '{"type":"text","text":"world"}');
    "#,
    )
    .unwrap();
    if !native {
      db.execute_batch("update session set time_updated = 2;").unwrap();
    }
    db.execute_batch("commit;").unwrap();
    let update = snapshot(&mut first).await;
    assert_eq!(update.generation, initial.generation);
    assert_eq!(messages(&update), ["hello", "world"]);
    assert_eq!(messages(&snapshot(&mut second).await), ["hello", "world"]);
    if !native {
      db.execute_batch("update session set title = 'New title';").unwrap();
      let metadata = snapshot(&mut first).await;
      assert_eq!(metadata.generation, initial.generation);
      assert_eq!(metadata.loaded.reference.title.as_deref(), Some("New title"));
      assert_eq!(messages(&metadata), ["hello", "world"]);
    }
    let mut reconnect = RelaySubscription::connect(&server.endpoint, &key).await.unwrap();
    let current = snapshot(&mut reconnect).await;
    assert_eq!(current.generation, initial.generation);
    assert_eq!(messages(&current), ["hello", "world"]);
  }
}

#[tokio::test]
async fn codex_turn_boundaries_reach_follow_clients_without_a_final_message() {
  let root = TempDir::new().unwrap();
  let path = root.path().join("rollout-turn.jsonl");
  std::fs::write(&path, concat!(
    "{\"type\":\"session_meta\",\"payload\":{\"id\":\"turn-session\"}}\n",
    "{\"type\":\"event_msg\",\"timestamp\":\"2026-09-03T00:00:00Z\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"t1\"}}\n"
  )).unwrap();
  let server = server(root.path(), Provider::Codex, true).await;
  let key = load_catalog(&server.endpoint).await.unwrap().entries.remove(0).key;
  let mut client = RelaySubscription::connect(&server.endpoint, &key).await.unwrap();
  let initial = snapshot(&mut client).await;
  assert!(initial.loaded.events.iter().any(
    |e| matches!(e, AgentEvent::Lifecycle(l) if l.turn_id == "t1" && l.phase == tokn_session_core::Phase::Started)
  ));
  append(
    &path,
    "{\"type\":\"event_msg\",\"timestamp\":\"2026-09-03T00:00:03Z\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"t1\"}}\n",
  );
  let finished = snapshot(&mut client).await;
  assert_eq!(initial.generation, finished.generation);
  assert!(
    matches!(finished.loaded.events.last(), Some(AgentEvent::Lifecycle(l)) if l.phase == tokn_session_core::Phase::Finished)
  );
}

#[tokio::test]
async fn framing_rejects_oversized_lengths_before_allocating_payloads() {
  let (mut writer, mut reader) = tokio::io::duplex(8);
  writer.write_u32(MAX_FRAME_BYTES as u32 + 1).await.unwrap();
  assert!(
    read_frame::<Frame>(&mut reader)
      .await
      .unwrap_err()
      .contains("size limit")
  );
  writer.write_u32(0).await.unwrap();
  assert!(read_frame::<Frame>(&mut reader).await.is_err());
}

#[tokio::test]
async fn codex_follow_retains_normalizer_state_across_native_only_updates() {
  let root = TempDir::new().unwrap();
  let path = root.path().join("rollout-accounting.jsonl");
  let counters = serde_json::json!({ "input_tokens": 100, "cached_input_tokens": 20, "output_tokens": 5, "reasoning_output_tokens": 2, "total_tokens": 105 });
  let record = serde_json::json!({ "type": "event_msg", "payload": { "type": "token_count", "info": { "total_token_usage": counters, "last_token_usage": counters }, "rate_limits": null } });
  let line = format!("{record}\n");
  std::fs::write(
    &path,
    format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"accounting\"}}}}\n{line}"),
  )
  .unwrap();
  let server = server(root.path(), Provider::Codex, true).await;
  let key = load_catalog(&server.endpoint).await.unwrap().entries.remove(0).key;
  let mut subscription = RelaySubscription::connect(&server.endpoint, &key).await.unwrap();
  let initial = snapshot(&mut subscription).await;
  assert_eq!(
    initial
      .loaded
      .events
      .iter()
      .filter(|e| matches!(e, AgentEvent::Usage(_)))
      .count(),
    1
  );
  append(&path, &line);
  let next = snapshot(&mut subscription).await;
  assert_eq!(next.generation, initial.generation);
  assert_ne!(next.revision, initial.revision);
  assert_eq!(
    next.loaded.events.len(),
    initial.loaded.events.len(),
    "unchanged accounting remains a native-only source record"
  );
}

#[test]
fn endpoint_requires_numeric_loopback_tcp() {
  for value in [
    "tcp://0.0.0.0:5557",
    "tcp://192.168.1.2:5557",
    "https://localhost:5557",
    "tcp://localhost:5557",
  ] {
    assert!(local_endpoint(value).is_err());
  }
  assert!(local_endpoint("tcp://127.0.0.1:5557").is_ok());
  assert!(local_endpoint("tcp://[::1]:5557").is_ok());
}
