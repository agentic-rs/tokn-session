use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher, event::ModifyKind};
use tokio::sync::mpsc;
use tokio::time::Instant;

use tokn_session_core::Provider;

use crate::{ProviderRoot, SessionTailer, TailUpdate};

/// Default interval for recovering from missed filesystem notifications and
/// discovering roots created after startup.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(30);
/// Default number of recent messages replayed from a newly discovered session.
pub const DEFAULT_REPLAY_MESSAGES: usize = 3;

/// History emitted when a session file is discovered or replaced after startup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NewFileReplay {
  /// Emit all complete records.
  All,
  /// Emit events beginning at the specified most-recent message.
  Messages(usize),
}

#[derive(Clone, Debug)]
pub struct RelayConfig {
  /// Provider session trees to follow.
  pub roots: Vec<ProviderRoot>,
  /// Interval for the fallback filesystem rescan.
  pub poll_interval: Duration,
  /// History emitted for a newly discovered or replaced session file.
  pub new_file_replay: NewFileReplay,
  /// Include provider-native records in the wire envelope (off by default).
  pub include_native: bool,
}

impl RelayConfig {
  pub fn new(roots: Vec<ProviderRoot>) -> Self {
    Self {
      roots,
      poll_interval: DEFAULT_POLL_INTERVAL,
      new_file_replay: NewFileReplay::Messages(DEFAULT_REPLAY_MESSAGES),
      include_native: false,
    }
  }
}

pub struct SessionRelay {
  tailer: SessionTailer,
  watcher: RecommendedWatcher,
  watched_paths: HashSet<PathBuf>,
  wake_rx: mpsc::UnboundedReceiver<Result<WatcherWake, String>>,
  poll: tokio::time::Interval,
  initial: Option<TailUpdate>,
}

impl SessionRelay {
  /// Creates a relay and starts watching all provider paths that already exist.
  pub async fn new(config: RelayConfig) -> Result<Self, String> {
    if config.poll_interval.is_zero() {
      return Err("relay poll interval must be greater than zero".to_string());
    }

    let mut tailer = SessionTailer::prepare(config.roots, config.new_file_replay)?;
    tailer.set_include_native(config.include_native);
    let (wake_tx, wake_rx) = mpsc::unbounded_channel();
    let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
      let result = result
        .map(|event| {
          let need_rescan = event.need_rescan();
          WatcherWake {
            paths: event.paths,
            kind: event.kind,
            need_rescan,
          }
        })
        .map_err(|err| err.to_string());
      let _ = wake_tx.send(result);
    })
    .map_err(|err| format!("failed to create filesystem watcher: {err}"))?;
    let mut relay = Self {
      tailer,
      watcher,
      watched_paths: HashSet::new(),
      wake_rx,
      poll: tokio::time::interval_at(Instant::now() + config.poll_interval, config.poll_interval),
      initial: None,
    };
    relay
      .poll
      .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    relay.watch_available_roots()?;
    relay.initial = Some(relay.tailer.start()?);
    Ok(relay)
  }

  /// Returns the configured provider roots.
  pub fn roots(&self) -> &[ProviderRoot] {
    self.tailer.roots()
  }

  /// Waits for a filesystem notification or fallback poll, then returns all
  /// newly normalized events and recoverable warnings.
  pub async fn next_update(&mut self) -> Result<TailUpdate, String> {
    if let Some(initial) = self.initial.take() {
      return Ok(initial);
    }

    let wake = tokio::select! {
      _ = self.poll.tick() => ScanRequest::Full,
      wake = self.wake_rx.recv() => {
        match wake {
          Some(wake) => ScanRequest::Watcher(wake),
          None => return Err("filesystem watcher stopped unexpectedly".to_string()),
        }
      }
    };

    let (scan_all, paths, mut watcher_warnings) = match wake {
      ScanRequest::Full => (true, HashSet::new(), Vec::new()),
      ScanRequest::Watcher(first) => self.collect_watcher_events(first),
    };
    let scan = if scan_all {
      self.tailer.scan()
    } else {
      self.tailer.scan_paths(paths)
    };
    let mut update = match scan {
      Ok(update) => update,
      Err(err) => TailUpdate {
        records: Vec::new(),
        warnings: vec![err],
      },
    };
    watcher_warnings.append(&mut update.warnings);
    update.warnings = watcher_warnings;
    if let Err(err) = self.watch_available_roots() {
      update.warnings.push(err);
    }
    Ok(update)
  }

  fn collect_watcher_events(&mut self, first: Result<WatcherWake, String>) -> (bool, HashSet<PathBuf>, Vec<String>) {
    let mut scan_all = false;
    let mut paths = HashSet::new();
    let mut warnings = Vec::new();
    let mut wakes = vec![first];
    while let Ok(wake) = self.wake_rx.try_recv() {
      wakes.push(wake);
    }
    for wake in wakes {
      match wake {
        Ok(event) => {
          merge_watcher_event(&mut self.watched_paths, &mut scan_all, &mut paths, event);
        }
        Err(err) => {
          scan_all = true;
          warnings.push(format!("filesystem watcher error: {err}"));
        }
      }
    }
    (scan_all, paths, warnings)
  }

  fn watch_available_roots(&mut self) -> Result<(), String> {
    for root in self.tailer.roots() {
      for (path, mode) in watch_targets(root) {
        if !path.exists() || self.watched_paths.contains(&path) {
          continue;
        }
        self
          .watcher
          .watch(&path, mode)
          .map_err(|err| format!("failed to watch {}: {err}", path.display()))?;
        self.watched_paths.insert(path);
      }
    }
    Ok(())
  }
}

#[derive(Debug)]
struct WatcherWake {
  paths: Vec<PathBuf>,
  kind: EventKind,
  need_rescan: bool,
}

enum ScanRequest {
  Full,
  Watcher(Result<WatcherWake, String>),
}

fn merge_watcher_event(
  watched_paths: &mut HashSet<PathBuf>,
  scan_all: &mut bool,
  paths: &mut HashSet<PathBuf>,
  event: WatcherWake,
) {
  if matches!(
    event.kind,
    EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
  ) {
    for path in &event.paths {
      watched_paths.remove(path);
    }
  }
  *scan_all |= event.need_rescan;
  paths.extend(event.paths);
}

fn watch_targets(root: &ProviderRoot) -> Vec<(PathBuf, RecursiveMode)> {
  if !matches!(root.provider, Provider::OpenCode) {
    if !root.path.exists() {
      return Vec::new();
    }
    let mode = if root.path.is_dir() {
      RecursiveMode::Recursive
    } else {
      RecursiveMode::NonRecursive
    };
    return vec![(root.path.clone(), mode)];
  }

  let database_path = if root.path.is_dir() {
    root.path.join("opencode.db")
  } else {
    root.path.clone()
  };
  let mut targets = Vec::new();
  if root.path.is_dir() {
    targets.push((root.path.clone(), RecursiveMode::NonRecursive));
  } else if let Some(parent) = database_path.parent().filter(|parent| parent.exists()) {
    targets.push((parent.to_path_buf(), RecursiveMode::NonRecursive));
  }

  // SQLite readers may update the SHM index themselves. Watching it feeds the
  // resulting notification back into another read; the database and WAL are
  // the durable change signals we need.
  for path in [database_path.clone(), sqlite_sidecar_path(&database_path, "-wal")] {
    if path.exists() {
      targets.push((path, RecursiveMode::NonRecursive));
    }
  }
  targets
}

fn sqlite_sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
  let name = database_path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or("opencode.db");
  database_path.with_file_name(format!("{name}{suffix}"))
}

#[cfg(test)]
mod tests {
  use std::fs::OpenOptions;
  use std::io::Write;
  use std::time::Duration;

  use notify::{EventKind, RecursiveMode};
  use rusqlite::{Connection, params};
  use tempfile::TempDir;
  use tokn_session_core::{AgentEvent, Provider};

  use super::{RelayConfig, SessionRelay, WatcherWake, merge_watcher_event, watch_targets};
  use crate::ProviderRoot;

  #[test]
  fn ordinary_empty_watcher_events_do_not_force_a_full_scan() {
    let mut watched_paths = std::collections::HashSet::new();
    let mut scan_all = false;
    let mut paths = std::collections::HashSet::new();
    merge_watcher_event(
      &mut watched_paths,
      &mut scan_all,
      &mut paths,
      WatcherWake {
        paths: Vec::new(),
        kind: EventKind::Other,
        need_rescan: false,
      },
    );
    assert!(!scan_all);
    assert!(paths.is_empty());
  }

  #[test]
  fn opencode_directory_watches_are_non_recursive_and_database_scoped() {
    let fixture = TempDir::new().unwrap();
    let database = fixture.path().join("opencode.db");
    std::fs::write(&database, b"not a database").unwrap();
    std::fs::write(fixture.path().join("opencode.db-wal"), b"wal").unwrap();
    std::fs::write(fixture.path().join("opencode.db-shm"), b"shm").unwrap();
    std::fs::write(fixture.path().join("opencode.log"), b"log").unwrap();

    let root = ProviderRoot::new(Provider::OpenCode, fixture.path().to_path_buf());
    let targets = watch_targets(&root);
    assert!(targets.iter().all(|(_, mode)| *mode == RecursiveMode::NonRecursive));
    assert!(targets.iter().any(|(path, _)| path == fixture.path()));
    assert!(targets.iter().any(|(path, _)| path == &database));
    assert!(
      targets
        .iter()
        .any(|(path, _)| path == &fixture.path().join("opencode.db-wal"))
    );
    assert!(
      !targets
        .iter()
        .any(|(path, _)| path == &fixture.path().join("opencode.db-shm"))
    );
    assert!(!targets.iter().any(|(path, _)| path.ends_with("opencode.log")));
  }

  #[tokio::test]
  async fn rejects_historical_only_providers_until_live_readers_exist() {
    for (provider, name) in [
      (Provider::Dsh, "dsh"),
      (Provider::ZCode, "zcode"),
      (Provider::WorkBuddy, "workbuddy"),
    ] {
      let config = RelayConfig::new(vec![ProviderRoot::new(provider, "unused".into())]);
      let error = SessionRelay::new(config)
        .await
        .err()
        .expect("historical-only relay provider must be rejected");
      assert!(error.contains(&format!("{name} relay watching is not implemented")));
    }
  }

  #[tokio::test]
  async fn follows_appends_through_the_library_api() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("session_test.jsonl");
    std::fs::write(
      &path,
      "{\"type\":\"session\",\"id\":\"pi-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
    )
    .unwrap();
    let mut config = RelayConfig::new(vec![ProviderRoot::new(Provider::Pi, fixture.path().to_path_buf())]);
    config.poll_interval = Duration::from_millis(10);
    let mut relay = SessionRelay::new(config).await.unwrap();
    assert!(relay.next_update().await.unwrap().records.is_empty());

    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file
      .write_all(b"{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n")
      .unwrap();
    file.flush().unwrap();

    let update = tokio::time::timeout(Duration::from_secs(2), async {
      loop {
        let update = relay.next_update().await.unwrap();
        if !update.records.is_empty() {
          break update;
        }
      }
    })
    .await
    .expect("relay timed out");
    assert_eq!(update.records.len(), 1);
    let AgentEvent::Message(message) = &update.records[0].record.events[0] else {
      panic!("expected message");
    };
    assert_eq!(message.text, "hello");
  }

  #[tokio::test]
  async fn discovers_a_provider_root_created_after_startup() {
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().join("sessions");
    let mut config = RelayConfig::new(vec![ProviderRoot::new(Provider::Pi, root.clone())]);
    config.poll_interval = Duration::from_millis(10);
    let mut relay = SessionRelay::new(config).await.unwrap();
    assert!(relay.next_update().await.unwrap().records.is_empty());

    std::fs::create_dir(&root).unwrap();
    std::fs::write(
      root.join("session_new.jsonl"),
      concat!(
        "{\"type\":\"session\",\"id\":\"new-session\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n",
        "{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n"
      ),
    )
    .unwrap();

    let update = tokio::time::timeout(Duration::from_secs(2), async {
      loop {
        let update = relay.next_update().await.unwrap();
        if !update.records.is_empty() {
          break update;
        }
      }
    })
    .await
    .expect("relay timed out");
    assert_eq!(update.records.len(), 2);
    assert!(update.records.iter().all(|event| event.topic == "pi.new-session"));
  }

  #[tokio::test]
  async fn rejects_zero_poll_interval() {
    let mut config = RelayConfig::new(Vec::new());
    config.poll_interval = Duration::ZERO;
    assert!(SessionRelay::new(config).await.is_err());
  }

  #[tokio::test]
  async fn follows_opencode_database_sessions_and_part_updates() {
    let fixture = TempDir::new().unwrap();
    let database = fixture.path().join("opencode.db");
    let connection = Connection::open(&database).unwrap();
    connection
      .execute_batch(
        "pragma journal_mode = wal;
         create table session (
           id text primary key,
           parent_id text,
           directory text not null,
           time_created integer not null,
           time_updated integer not null
         );
         create table message (
           id text primary key,
           session_id text not null,
           time_created integer,
           data text not null
         );
         create table part (
           id text primary key,
           message_id text not null,
           session_id text not null,
           time_created integer,
           data text not null
         );",
      )
      .unwrap();
    drop(connection);
    let mut config = RelayConfig::new(vec![ProviderRoot::new(Provider::OpenCode, database.clone())]);
    config.poll_interval = Duration::from_millis(10);
    let mut relay = SessionRelay::new(config).await.unwrap();
    assert!(relay.next_update().await.unwrap().records.is_empty());

    let connection = Connection::open(&database).unwrap();
    insert_session(&connection, "ses_1", 1, 2);
    insert_message(&connection, "msg_user", "ses_1", 1, r#"{"role":"user"}"#);
    insert_part(
      &connection,
      "part_user",
      "msg_user",
      "ses_1",
      1,
      r#"{"type":"text","text":"hello"}"#,
    );
    let first = wait_for_events(&mut relay).await;
    assert_eq!(first.len(), 2);
    assert!(first.iter().all(|event| event.topic == "opencode.ses_1"));
    assert!(
      first
        .iter()
        .any(|event| matches!(event.record.events[0], AgentEvent::SessionStarted(_)))
    );
    assert!(
      first
        .iter()
        .any(|event| matches!(event.record.events[0], AgentEvent::Message(_)))
    );

    insert_message(
      &connection,
      "msg_assistant",
      "ses_1",
      3,
      r#"{"role":"assistant","parentID":"msg_user"}"#,
    );
    insert_part(
      &connection,
      "part_assistant",
      "msg_assistant",
      "ses_1",
      3,
      r#"{"type":"text","text":"world"}"#,
    );
    let second = wait_for_events(&mut relay).await;
    assert_eq!(second.len(), 1);
    let AgentEvent::Message(message) = &second[0].record.events[0] else {
      panic!("expected assistant message");
    };
    assert_eq!(message.text, "world");

    connection
      .execute(
        "update part set data = ?1 where id = ?2",
        params![r#"{"type":"text","text":"updated"}"#, "part_assistant"],
      )
      .unwrap();
    let third = wait_for_events(&mut relay).await;
    assert_eq!(third.len(), 1);
    let AgentEvent::Message(message) = &third[0].record.events[0] else {
      panic!("expected updated assistant message");
    };
    assert_eq!(message.text, "updated");
  }

  async fn wait_for_events(relay: &mut SessionRelay) -> Vec<crate::RelayRecord> {
    tokio::time::timeout(Duration::from_secs(2), async {
      loop {
        let update = relay.next_update().await.unwrap();
        if !update.records.is_empty() {
          break update.records;
        }
      }
    })
    .await
    .expect("relay timed out")
  }

  fn insert_session(connection: &Connection, id: &str, time_created: i64, time_updated: i64) {
    connection
      .execute(
        "insert into session (id, parent_id, directory, time_created, time_updated) values (?1, null, ?2, ?3, ?4)",
        params![id, "/tmp/opencode", time_created, time_updated],
      )
      .unwrap();
  }

  fn insert_message(connection: &Connection, id: &str, session_id: &str, time_created: i64, data: &str) {
    connection
      .execute(
        "insert into message (id, session_id, time_created, data) values (?1, ?2, ?3, ?4)",
        params![id, session_id, time_created, data],
      )
      .unwrap();
  }

  fn insert_part(connection: &Connection, id: &str, message_id: &str, session_id: &str, time_created: i64, data: &str) {
    connection
      .execute(
        "insert into part (id, message_id, session_id, time_created, data) values (?1, ?2, ?3, ?4, ?5)",
        params![id, message_id, session_id, time_created, data],
      )
      .unwrap();
  }
}
