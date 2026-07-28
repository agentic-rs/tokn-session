use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::{ProviderRoot, SessionTailer, TailUpdate};

/// Default interval for recovering from missed filesystem notifications and
/// discovering roots created after startup.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub struct RelayConfig {
  /// Provider session trees to follow.
  pub roots: Vec<ProviderRoot>,
  /// Whether records present at startup should be emitted.
  pub replay: bool,
  /// Interval for the fallback filesystem rescan.
  pub poll_interval: Duration,
}

impl RelayConfig {
  pub fn new(roots: Vec<ProviderRoot>) -> Self {
    Self {
      roots,
      replay: false,
      poll_interval: DEFAULT_POLL_INTERVAL,
    }
  }
}

pub struct SessionRelay {
  tailer: SessionTailer,
  watcher: RecommendedWatcher,
  watched_roots: HashSet<PathBuf>,
  wake_rx: mpsc::UnboundedReceiver<Result<(), String>>,
  poll: tokio::time::Interval,
  initial: Option<TailUpdate>,
}

impl SessionRelay {
  /// Creates a relay and starts watching all provider roots that already exist.
  pub async fn new(config: RelayConfig) -> Result<Self, String> {
    if config.poll_interval.is_zero() {
      return Err("relay poll interval must be greater than zero".to_string());
    }

    let (tailer, initial) = SessionTailer::initialize(config.roots, config.replay)?;
    let (wake_tx, wake_rx) = mpsc::unbounded_channel();
    let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
      let result = result.map(|_| ()).map_err(|err| err.to_string());
      let _ = wake_tx.send(result);
    })
    .map_err(|err| format!("failed to create filesystem watcher: {err}"))?;
    let mut relay = Self {
      tailer,
      watcher,
      watched_roots: HashSet::new(),
      wake_rx,
      poll: tokio::time::interval(config.poll_interval),
      initial: Some(initial),
    };
    relay
      .poll
      .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    relay.watch_available_roots()?;
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

    let watcher_warning = tokio::select! {
      _ = self.poll.tick() => None,
      wake = self.wake_rx.recv() => {
        match wake {
          Some(Ok(())) => None,
          Some(Err(err)) => Some(format!("filesystem watcher error: {err}")),
          None => return Err("filesystem watcher stopped unexpectedly".to_string()),
        }
      }
    };

    let mut update = match self.tailer.scan() {
      Ok(update) => update,
      Err(err) => TailUpdate {
        events: Vec::new(),
        warnings: vec![err],
      },
    };
    if let Some(warning) = watcher_warning {
      update.warnings.insert(0, warning);
    }
    if let Err(err) = self.watch_available_roots() {
      update.warnings.push(err);
    }
    Ok(update)
  }

  fn watch_available_roots(&mut self) -> Result<(), String> {
    for root in self.tailer.roots() {
      if !root.path.exists() || self.watched_roots.contains(&root.path) {
        continue;
      }
      self
        .watcher
        .watch(&root.path, RecursiveMode::Recursive)
        .map_err(|err| format!("failed to watch {}: {err}", root.path.display()))?;
      self.watched_roots.insert(root.path.clone());
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use std::fs::OpenOptions;
  use std::io::Write;
  use std::time::Duration;

  use tempfile::TempDir;
  use tokn_session_core::{AgentEvent, Provider};

  use super::{RelayConfig, SessionRelay};
  use crate::ProviderRoot;

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
    assert!(relay.next_update().await.unwrap().events.is_empty());

    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file
      .write_all(b"{\"type\":\"message\",\"id\":\"new\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n")
      .unwrap();
    file.flush().unwrap();

    let update = tokio::time::timeout(Duration::from_secs(2), async {
      loop {
        let update = relay.next_update().await.unwrap();
        if !update.events.is_empty() {
          break update;
        }
      }
    })
    .await
    .expect("relay timed out");
    assert_eq!(update.events.len(), 1);
    let AgentEvent::Message(message) = &update.events[0].event else {
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
    assert!(relay.next_update().await.unwrap().events.is_empty());

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
        if !update.events.is_empty() {
          break update;
        }
      }
    })
    .await
    .expect("relay timed out");
    assert_eq!(update.events.len(), 2);
    assert!(update.events.iter().all(|event| event.topic == "pi.new-session"));
  }

  #[tokio::test]
  async fn rejects_zero_poll_interval() {
    let mut config = RelayConfig::new(Vec::new());
    config.poll_interval = Duration::ZERO;
    assert!(SessionRelay::new(config).await.is_err());
  }
}
