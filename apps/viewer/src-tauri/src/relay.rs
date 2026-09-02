//! Connection-only viewer adapter. Covered providers never fall back to local
//! body reads while a Relay catalog is authoritative, including reconnects.
use std::{
  collections::HashMap,
  path::PathBuf,
  sync::{Arc, Condvar, Mutex},
  time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tokn_session_core::{LoadedSession, Provider, SessionHeader};
use tokn_session_relay::{
  service_client::{RelaySubscription, load_catalog},
  service_protocol::{CatalogEntry, DEFAULT_SERVICE_ENDPOINT, local_endpoint},
};

use crate::model::{SessionLocator, ViewerProvider, encode_session_key};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RelaySettings {
  pub endpoint: String,
  pub enabled: bool,
}
impl Default for RelaySettings {
  fn default() -> Self {
    Self {
      endpoint: DEFAULT_SERVICE_ENDPOINT.into(),
      enabled: false,
    }
  }
}

#[derive(Clone, Debug, Serialize)]
pub struct RelayStatus {
  pub settings: RelaySettings,
  pub phase: String,
  pub native: bool,
  pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RelayChange {
  pub session_key: Option<String>,
  pub reset: bool,
}

struct CachedSession {
  loaded: Option<Arc<LoadedSession>>,
  native: Vec<Option<Arc<serde_json::Value>>>,
  displayed: Option<Arc<LoadedSession>>,
  displayed_native: Vec<Option<Arc<serde_json::Value>>>,
  error: Option<String>,
  generation: String,
  cancel: CancellationToken,
  accessed: Instant,
}

struct State {
  settings: RelaySettings,
  phase: String,
  error: Option<String>,
  native: bool,
  epoch: u64,
  cancel: CancellationToken,
  providers: Vec<ViewerProvider>,
  entries: Option<Vec<CatalogEntry>>,
  sessions: HashMap<SessionLocator, CachedSession>,
}

pub struct ViewerRelay {
  pub configure_lock: tokio::sync::Mutex<()>,
  state: Mutex<State>,
  ready: Condvar,
  pub changes: broadcast::Sender<RelayChange>,
}

impl ViewerRelay {
  pub fn new() -> Arc<Self> {
    Arc::new(Self {
      configure_lock: tokio::sync::Mutex::new(()),
      state: Mutex::new(State {
        settings: RelaySettings::default(),
        phase: "disconnected".into(),
        error: None,
        native: false,
        epoch: 0,
        cancel: CancellationToken::new(),
        providers: Vec::new(),
        entries: None,
        sessions: HashMap::new(),
      }),
      ready: Condvar::new(),
      changes: broadcast::channel(128).0,
    })
  }

  pub fn status(&self) -> RelayStatus {
    let state = self.state.lock().unwrap();
    let error = state
      .error
      .clone()
      .or_else(|| state.sessions.values().find_map(|session| session.error.clone()));
    RelayStatus {
      settings: state.settings.clone(),
      phase: if error.is_some() && state.settings.enabled {
        "reconnecting".into()
      } else {
        state.phase.clone()
      },
      native: state.native,
      error,
    }
  }

  pub fn configure(self: &Arc<Self>, settings: RelaySettings) -> Result<(), String> {
    local_endpoint(&settings.endpoint)?;
    let (epoch, cancel, reset) = {
      let mut state = self.state.lock().unwrap();
      state.cancel.cancel();
      for session in state.sessions.values() {
        session.cancel.cancel();
      }
      // Disconnect retains last-good data; switching endpoint must never mix
      // snapshots from independently configured services.
      let reset = state.settings.endpoint != settings.endpoint || state.entries.is_none();
      if state.settings.endpoint != settings.endpoint {
        state.entries = None;
        state.providers.clear();
        state.sessions.clear();
      }
      if settings.enabled && state.entries.is_none() {
        state.providers = vec![ViewerProvider::Codex, ViewerProvider::Pi, ViewerProvider::OpenCode];
      }
      if !settings.enabled && state.entries.is_none() {
        state.providers.clear();
      }
      state.epoch += 1;
      state.cancel = CancellationToken::new();
      state.settings = settings.clone();
      state.phase = if settings.enabled { "connecting" } else { "disconnected" }.into();
      state.error = None;
      self.ready.notify_all();
      (state.epoch, state.cancel.clone(), reset)
    };
    self.notify(None, reset);
    if settings.enabled {
      let manager = self.clone();
      tauri::async_runtime::spawn(async move {
        manager.catalog_loop(settings.endpoint, epoch, cancel).await;
      });
    }
    Ok(())
  }

  async fn catalog_loop(self: Arc<Self>, endpoint: String, epoch: u64, cancel: CancellationToken) {
    loop {
      let result = tokio::select! {
        _ = cancel.cancelled() => return,
        result = load_catalog(&endpoint) => result,
      };
      let (changed, first_catalog, resume) = {
        let mut state = self.state.lock().unwrap();
        if state.epoch != epoch {
          return;
        }
        let before = serde_json::to_vec(&state.entries).ok();
        let had_catalog = state.entries.is_some();
        let was_live = state.phase == "live";
        let old_error = state.error.clone();
        match result {
          Ok(catalog) => {
            state.providers = catalog.providers.iter().filter_map(|p| viewer_provider(*p)).collect();
            state.native = catalog.native;
            // Catalog failures keep the last complete catalog rather than
            // presenting partial discovery as authoritative deletion.
            if catalog.warnings.is_empty() || state.entries.is_none() {
              state.entries = Some(catalog.entries);
            }
            state.error = (!catalog.warnings.is_empty()).then(|| catalog.warnings.join("; "));
            state.phase = "live".into();
          }
          Err(error) => {
            state.phase = "reconnecting".into();
            state.error = Some(error);
          }
        }
        let became_live = !was_live && state.phase == "live";
        let resume: Vec<_> = if became_live {
          state
            .sessions
            .iter()
            .filter(|(_, s)| s.cancel.is_cancelled())
            .map(|(key, _)| key.clone())
            .collect()
        } else {
          Vec::new()
        };
        (
          before != serde_json::to_vec(&state.entries).ok() || became_live || old_error != state.error,
          !had_catalog && state.entries.is_some(),
          resume,
        )
      };
      self.ready.notify_all();
      if changed {
        self.notify(None, first_catalog);
      }
      for locator in resume {
        let manager = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
          let _ = manager.load(&locator);
        });
      }
      tokio::select! { _ = cancel.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(3)) => {} }
    }
  }

  pub fn covers(&self, provider: ViewerProvider) -> bool {
    self.state.lock().unwrap().providers.contains(&provider)
  }

  pub fn has_catalog(&self) -> bool {
    self.state.lock().unwrap().entries.is_some()
  }

  pub fn headers(&self, provider: ViewerProvider) -> Vec<SessionHeader> {
    self
      .state
      .lock()
      .unwrap()
      .entries
      .as_ref()
      .into_iter()
      .flatten()
      .filter(|entry| viewer_provider(entry.provider) == Some(provider))
      .map(|entry| entry.header.clone())
      .collect()
  }

  pub fn load(self: &Arc<Self>, locator: &SessionLocator) -> Result<Arc<LoadedSession>, String> {
    let mut state = self.state.lock().unwrap();
    let should_start = state.settings.enabled && state.sessions.get(locator).is_none_or(|s| s.cancel.is_cancelled());
    if should_start {
      let entry = state
        .entries
        .as_ref()
        .into_iter()
        .flatten()
        .find(|e| matches_locator(e, locator))
        .cloned()
        .ok_or("Session is no longer in the Relay catalog")?;
      if !state.sessions.contains_key(locator)
        && state.sessions.len() >= 8
        && let Some(key) = state
          .sessions
          .iter()
          .min_by_key(|(_, session)| session.accessed)
          .map(|(key, _)| key.clone())
      {
        if let Some(old) = state.sessions.remove(&key) {
          old.cancel.cancel();
        }
      }
      let cancel = state.cancel.child_token();
      let mut old = state.sessions.remove(locator);
      state.sessions.insert(
        locator.clone(),
        CachedSession {
          loaded: old.as_ref().and_then(|s| s.loaded.clone()),
          native: old.as_mut().map(|s| std::mem::take(&mut s.native)).unwrap_or_default(),
          displayed: old.as_ref().and_then(|s| s.displayed.clone()),
          displayed_native: old
            .as_mut()
            .map(|s| std::mem::take(&mut s.displayed_native))
            .unwrap_or_default(),
          error: None,
          generation: old.map(|s| s.generation).unwrap_or_default(),
          cancel: cancel.clone(),
          accessed: Instant::now(),
        },
      );
      let manager = self.clone();
      let endpoint = state.settings.endpoint.clone();
      let epoch = state.epoch;
      let locator = locator.clone();
      tauri::async_runtime::spawn(async move {
        manager.session_loop(endpoint, entry.key, locator, epoch, cancel).await;
      });
    }
    let deadline = Instant::now() + Duration::from_secs(12);
    let epoch = state.epoch;
    loop {
      if state.epoch != epoch {
        return Err("Relay connection changed; reload the session".into());
      }
      let session = state
        .sessions
        .get_mut(locator)
        .ok_or("Relay is disconnected; connect to load this session")?;
      session.accessed = Instant::now();
      if session.displayed.is_none() {
        session.displayed = session.loaded.clone();
        session.displayed_native = session.native.clone();
      }
      if let Some(loaded) = &session.displayed {
        return Ok(loaded.clone());
      }
      if let Some(error) = &session.error {
        return Err(error.clone());
      }
      let remaining = deadline.saturating_duration_since(Instant::now());
      if remaining.is_zero() {
        return Err("Waiting for Relay snapshot; retry when connected".into());
      }
      state = self.ready.wait_timeout(state, remaining).unwrap().0;
    }
  }

  async fn session_loop(
    self: Arc<Self>,
    endpoint: String,
    key: String,
    locator: SessionLocator,
    epoch: u64,
    cancel: CancellationToken,
  ) {
    loop {
      let result = tokio::select! {
        _ = cancel.cancelled() => return,
        result = self.consume_session(&endpoint, &key, &locator, epoch, &cancel) => result,
      };
      if let Err(error) = result {
        let mut state = self.state.lock().unwrap();
        if state.epoch != epoch || cancel.is_cancelled() {
          return;
        }
        if let Some(session) = state.sessions.get_mut(&locator) {
          session.error = Some(error);
        }
      }
      self.ready.notify_all();
      self.notify(None, false);
      tokio::select! { _ = cancel.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(2)) => {} }
    }
  }

  async fn consume_session(
    &self,
    endpoint: &str,
    key: &str,
    locator: &SessionLocator,
    epoch: u64,
    cancel: &CancellationToken,
  ) -> Result<(), String> {
    let mut subscription = RelaySubscription::connect(endpoint, key).await?;
    loop {
      let snapshot = subscription.next_snapshot().await?;
      let reset = {
        let mut state = self.state.lock().unwrap();
        if state.epoch != epoch || cancel.is_cancelled() {
          return Ok(());
        }
        let Some(session) = state.sessions.get_mut(locator) else {
          return Ok(());
        };
        let reset = !session.generation.is_empty() && session.generation != snapshot.generation;
        session.generation = snapshot.generation;
        session.loaded = Some(Arc::new(snapshot.loaded));
        session.native = snapshot.native;
        session.error = None;
        reset
      };
      self.ready.notify_all();
      self.notify(Some(locator), reset);
    }
  }

  pub fn native(
    &self,
    locator: &SessionLocator,
    index: usize,
    loaded: &Arc<LoadedSession>,
  ) -> Option<serde_json::Value> {
    let state = self.state.lock().ok()?;
    let session = state.sessions.get(locator)?;
    if !Arc::ptr_eq(session.displayed.as_ref()?, loaded) {
      return None;
    }
    session
      .displayed_native
      .get(index)?
      .as_ref()
      .map(|value| value.as_ref().clone())
  }

  /// Only newest-page requests advance the displayed snapshot. Pagination,
  /// expanded trajectories and Inspector requests remain on that same image.
  pub fn advance(self: &Arc<Self>, locator: &SessionLocator) -> Result<Arc<LoadedSession>, String> {
    self.load(locator)?;
    let mut state = self.state.lock().unwrap();
    let session = state
      .sessions
      .get_mut(locator)
      .ok_or("Relay connection changed; reload the session")?;
    session.displayed = session.loaded.clone();
    session.displayed_native = session.native.clone();
    session
      .displayed
      .clone()
      .ok_or_else(|| "Relay snapshot unavailable".into())
  }

  fn notify(&self, locator: Option<&SessionLocator>, reset: bool) {
    let session_key = locator.and_then(|locator| encode_session_key(locator).ok());
    let _ = self.changes.send(RelayChange { session_key, reset });
  }
}

fn matches_locator(entry: &CatalogEntry, locator: &SessionLocator) -> bool {
  viewer_provider(entry.provider) == Some(locator.provider)
    && entry.header.path == locator.source_path
    && entry.header.id == locator.session_id
}

fn viewer_provider(provider: Provider) -> Option<ViewerProvider> {
  match provider {
    Provider::Codex => Some(ViewerProvider::Codex),
    Provider::Pi => Some(ViewerProvider::Pi),
    Provider::OpenCode => Some(ViewerProvider::OpenCode),
    _ => None,
  }
}

pub fn read_settings(path: &PathBuf) -> Result<RelaySettings, String> {
  match std::fs::read(path) {
    Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("Invalid Relay settings: {e}")),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RelaySettings::default()),
    Err(e) => Err(e.to_string()),
  }
}

pub fn write_settings(path: &PathBuf, settings: &RelaySettings) -> Result<(), String> {
  local_endpoint(&settings.endpoint)?;
  let parent = path.parent().ok_or("Invalid settings path")?;
  std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
  let temporary = path.with_extension("json.tmp");
  std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
  std::fs::rename(temporary, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::{fs::OpenOptions, io::Write};
  use tokn_session_relay::{ProviderRoot, RelayConfig, service_server::serve_listener};

  #[test]
  fn saves_settings_and_rejects_nonlocal_endpoint_without_overwriting() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings/relay.json");
    assert!(!read_settings(&path).unwrap().enabled);
    write_settings(
      &path,
      &RelaySettings {
        endpoint: DEFAULT_SERVICE_ENDPOINT.into(),
        enabled: true,
      },
    )
    .unwrap();
    assert!(
      write_settings(
        &path,
        &RelaySettings {
          endpoint: "tcp://0.0.0.0:5557".into(),
          enabled: false
        }
      )
      .is_err()
    );
    assert!(read_settings(&path).unwrap().enabled);
  }

  #[tokio::test]
  async fn pinned_snapshot_survives_updates_disconnect_and_reconnect() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.jsonl");
    const HEADER: &str = "{\"type\":\"session\",\"id\":\"session-1\",\"timestamp\":\"2026-01-01\",\"cwd\":\"/tmp\"}\n";
    const MESSAGE: &str =
      "{\"type\":\"message\",\"id\":\"one\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n";
    std::fs::write(&path, format!("{HEADER}{MESSAGE}")).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("tcp://{}", listener.local_addr().unwrap());
    let mut config = RelayConfig::new(vec![ProviderRoot::new(Provider::Pi, root.path().into())]);
    config.include_native = true;
    config.poll_interval = Duration::from_millis(20);
    let server = tokio::spawn(serve_listener(listener, config));
    let manager = ViewerRelay::new();
    let mut changes = manager.changes.subscribe();
    manager
      .configure(RelaySettings {
        endpoint: endpoint.clone(),
        enabled: true,
      })
      .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
      while !manager.has_catalog() {
        changes.recv().await.unwrap();
      }
    })
    .await
    .unwrap();
    assert!(manager.covers(ViewerProvider::Pi));
    assert!(!manager.covers(ViewerProvider::Codex));
    let locator = SessionLocator {
      version: 1,
      provider: ViewerProvider::Pi,
      session_id: "session-1".into(),
      source_path: path.clone(),
    };
    let load_manager = manager.clone();
    let load_locator = locator.clone();
    let initial = tokio::task::spawn_blocking(move || load_manager.load(&load_locator))
      .await
      .unwrap()
      .unwrap();
    let initial_count = initial.events.len();
    OpenOptions::new()
      .append(true)
      .open(&path)
      .unwrap()
      .write_all(MESSAGE.replace("one", "two").as_bytes())
      .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
      loop {
        if manager.state.lock().unwrap().sessions[&locator]
          .loaded
          .as_ref()
          .unwrap()
          .events
          .len()
          > initial_count
        {
          break;
        }
        changes.recv().await.unwrap();
      }
    })
    .await
    .unwrap();
    assert!(
      Arc::ptr_eq(&initial, &manager.load(&locator).unwrap()),
      "details and paging retain the displayed snapshot"
    );
    let latest = manager.advance(&locator).unwrap();
    assert_eq!(latest.events.len(), initial_count + 1);
    assert!(manager.native(&locator, initial_count, &latest).is_some());
    assert!(
      manager.native(&locator, 0, &initial).is_none(),
      "native payload cannot cross snapshots"
    );
    manager
      .configure(RelaySettings {
        endpoint: endpoint.clone(),
        enabled: false,
      })
      .unwrap();
    assert_eq!(manager.status().phase, "disconnected");
    assert!(Arc::ptr_eq(&latest, &manager.load(&locator).unwrap()));
    manager
      .configure(RelaySettings {
        endpoint: endpoint.clone(),
        enabled: true,
      })
      .unwrap();
    OpenOptions::new()
      .append(true)
      .open(&path)
      .unwrap()
      .write_all(MESSAGE.replace("one", "three").as_bytes())
      .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
      loop {
        if manager.state.lock().unwrap().sessions[&locator]
          .loaded
          .as_ref()
          .unwrap()
          .events
          .len()
          > latest.events.len()
        {
          break;
        }
        changes.recv().await.unwrap();
      }
    })
    .await
    .unwrap();
    assert_eq!(manager.advance(&locator).unwrap().events.len(), initial_count + 2);
    manager
      .configure(RelaySettings {
        endpoint,
        enabled: false,
      })
      .unwrap();
    manager
      .configure(RelaySettings {
        endpoint: "tcp://127.0.0.1:1".into(),
        enabled: false,
      })
      .unwrap();
    assert!(!manager.covers(ViewerProvider::Pi));
    assert!(
      manager.load(&locator).is_err(),
      "different services never share cached snapshots"
    );
    server.abort();
  }
}
