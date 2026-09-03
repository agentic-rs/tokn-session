//! Exercise the actual shipped executable without starting a GUI or touching
//! the user's provider roots. These tests also run headlessly in Linux CI.
use std::{path::Path, process::Stdio, time::Duration};

use tokio::{
  io::{AsyncBufReadExt, BufReader},
  process::{Child, Command},
};
use tokn_session_relay::service_client::{RelaySubscription, load_catalog};

const BINARY: &str = env!("CARGO_BIN_EXE_tokn-session-viewer");
const CHILD_FLAG: &str = "--tokn-viewer-relay-child";
const HEADER: &str =
  "{\"type\":\"session\",\"id\":\"managed-fixture\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n";
const MESSAGE: &str = "{\"type\":\"message\",\"id\":\"one\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n";

fn command(root: &Path, native: bool) -> Command {
  std::fs::create_dir_all(root.join("codex/sessions")).unwrap();
  std::fs::create_dir_all(root.join("pi")).unwrap();
  let mut command = Command::new(std::env::var_os("TOKN_VIEWER_TEST_BINARY").unwrap_or_else(|| BINARY.into()));
  command
    .arg(CHILD_FLAG)
    .env("CODEX_HOME", root.join("codex"))
    .env("PI_CODING_AGENT_SESSION_DIR", root.join("pi"))
    .env("OPENCODE_DB", root.join("missing-opencode.db"))
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit())
    .kill_on_drop(true);
  if native {
    command.arg("--native");
  }
  command
}

async fn start(root: &Path, native: bool) -> (Child, String) {
  let mut child = command(root, native).spawn().unwrap();
  let mut line = String::new();
  tokio::time::timeout(
    Duration::from_secs(10),
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line),
  )
  .await
  .unwrap()
  .unwrap();
  let endpoint = serde_json::from_str::<Result<String, String>>(&line).unwrap().unwrap();
  (child, endpoint)
}

async fn stop(child: &mut Child) {
  drop(child.stdin.take());
  let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
    .await
    .expect("Relay did not exit after its parent closed stdin")
    .unwrap();
  assert!(status.success());
}

#[tokio::test]
async fn packaged_child_serves_fixtures_with_optional_native_and_exits_on_eof() {
  let root = tempfile::tempdir().unwrap();
  std::fs::create_dir(root.path().join("pi")).unwrap();
  std::fs::write(root.path().join("pi/session.jsonl"), format!("{HEADER}{MESSAGE}")).unwrap();
  for native in [false, true] {
    let (mut child, endpoint) = start(root.path(), native).await;
    let catalog = load_catalog(&endpoint).await.unwrap();
    assert_eq!(catalog.native, native);
    assert_eq!(catalog.entries.len(), 1);
    let mut subscription = RelaySubscription::connect(&endpoint, &catalog.entries[0].key)
      .await
      .unwrap();
    let snapshot = subscription.next_snapshot().await.unwrap();
    assert!(!snapshot.loaded.events.is_empty());
    assert_eq!(snapshot.native.iter().any(Option::is_some), native);
    stop(&mut child).await;
    assert!(load_catalog(&endpoint).await.is_err());
    assert!(subscription.next_snapshot().await.is_err());
  }
}

#[tokio::test]
async fn automatic_catalog_keeps_codex_titles_for_active_and_archived_roots() {
  let root = tempfile::tempdir().unwrap();
  let home = root.path().join("codex");
  for (directory, id) in [("sessions", "active-title"), ("archived_sessions", "archived-title")] {
    std::fs::create_dir_all(home.join(directory)).unwrap();
    let header = serde_json::json!({"type": "session_meta", "payload": {"id": id, "cwd": "/tmp"}});
    // A catalog must preserve metadata without parsing the transcript body.
    std::fs::write(
      home.join(directory).join("session.jsonl"),
      format!("{header}\nnot a valid transcript record\n"),
    )
    .unwrap();
  }
  std::fs::write(home.join("session_index.jsonl"), "{\"id\":\"active-title\",\"thread_name\":\"Active task title\"}\n{\"id\":\"archived-title\",\"thread_name\":\"Archived task title\"}\n").unwrap();
  let (mut child, endpoint) = start(root.path(), false).await;
  let catalog = load_catalog(&endpoint).await.unwrap();
  assert_eq!(catalog.entries.len(), 2);
  for (id, title) in [
    ("active-title", "Active task title"),
    ("archived-title", "Archived task title"),
  ] {
    let entry = catalog.entries.iter().find(|entry| entry.header.id == id).unwrap();
    assert_eq!(entry.header.title.as_deref(), Some(title));
  }
  stop(&mut child).await;
}

#[tokio::test]
async fn independent_children_do_not_share_ports_or_shutdown() {
  let root = tempfile::tempdir().unwrap();
  let (mut first, first_endpoint) = start(root.path(), false).await;
  let (mut second, second_endpoint) = start(root.path(), false).await;
  assert_ne!(first_endpoint, second_endpoint);
  stop(&mut first).await;
  assert!(load_catalog(&second_endpoint).await.is_ok());
  stop(&mut second).await;
}

#[tokio::test]
async fn invalid_child_configuration_reports_startup_error_without_opening_a_window() {
  let root = tempfile::tempdir().unwrap();
  let mut child = command(root.path(), false)
    .env("OPENCODE_DB", ":memory:")
    .spawn()
    .unwrap();
  let mut line = String::new();
  tokio::time::timeout(
    Duration::from_secs(5),
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line),
  )
  .await
  .unwrap()
  .unwrap();
  assert!(
    serde_json::from_str::<Result<String, String>>(&line)
      .unwrap()
      .unwrap_err()
      .contains(":memory:")
  );
  // wait() otherwise closes stdin and races the child's error exit with the
  // lifetime EOF handler's normal exit.
  let _lifetime = child.stdin.take();
  assert!(
    !tokio::time::timeout(Duration::from_secs(5), child.wait())
      .await
      .unwrap()
      .unwrap()
      .success()
  );
}

// Run only inside the parent-death test's helper process. Holding the child
// object keeps its lifetime pipe open; forcibly killing this helper skips Drop.
#[test]
fn parent_fixture() {
  let Some(root) = std::env::var_os("TOKN_RELAY_PARENT_FIXTURE_ROOT") else {
    return;
  };
  let runtime = tokio::runtime::Runtime::new().unwrap();
  let (_child, endpoint) = runtime.block_on(start(Path::new(&root), false));
  println!("CHILD_ENDPOINT={endpoint}");
  use std::io::Write;
  std::io::stdout().flush().unwrap();
  loop {
    std::thread::park();
  }
}

#[tokio::test]
async fn relay_exits_when_parent_dies_without_running_cleanup() {
  let root = tempfile::tempdir().unwrap();
  let mut parent = Command::new(std::env::current_exe().unwrap())
    .args(["--exact", "parent_fixture", "--nocapture"])
    .env("TOKN_RELAY_PARENT_FIXTURE_ROOT", root.path())
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit())
    .kill_on_drop(true)
    .spawn()
    .unwrap();
  let mut lines = BufReader::new(parent.stdout.take().unwrap()).lines();
  let endpoint = tokio::time::timeout(Duration::from_secs(10), async {
    loop {
      let line = lines.next_line().await.unwrap().expect("parent fixture exited");
      if let Some(endpoint) = line.strip_prefix("CHILD_ENDPOINT=") {
        break endpoint.to_owned();
      }
    }
  })
  .await
  .unwrap();
  assert!(load_catalog(&endpoint).await.is_ok());
  parent.kill().await.unwrap();
  tokio::time::timeout(Duration::from_secs(5), async {
    while tokio::net::TcpStream::connect(endpoint.trim_start_matches("tcp://"))
      .await
      .is_ok()
    {
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
  })
  .await
  .expect("orphaned Relay kept listening after parent death");
}
