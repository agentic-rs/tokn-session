use std::{
  collections::HashMap,
  sync::{Arc, Weak},
  time::Duration,
};

use tokio::{
  net::TcpListener,
  sync::{Mutex, Semaphore, watch},
};
use tokn_session_client::AgentClient;
use tokn_session_core::Provider;

use crate::{
  RelayConfig,
  service_metadata::PresentationCache,
  service_protocol::*,
  service_source::{SessionReader, Snapshot},
};

struct FollowedSession {
  current: watch::Sender<Arc<Snapshot>>,
}

pub struct Service {
  index: Option<Arc<tokn_session_index::SessionIndex>>,
  wake: watch::Sender<()>,
  config: RelayConfig,
  sessions: Mutex<HashMap<String, Weak<FollowedSession>>>,
  catalog: Mutex<Option<(std::time::Instant, Arc<Vec<CatalogEntry>>, Vec<String>)>>,
  metadata: Arc<PresentationCache>,
}

/// Serve independently configured local consumers. Session bodies are opened
/// for follow requests and a shared background title/preview backfill.
pub async fn serve(endpoint: &str, config: RelayConfig) -> Result<(), String> {
  let listener = TcpListener::bind(local_endpoint(endpoint)?)
    .await
    .map_err(|err| err.to_string())?;
  serve_listener(listener, config).await
}

pub async fn serve_listener(listener: TcpListener, config: RelayConfig) -> Result<(), String> {
  if config.poll_interval.is_zero() {
    return Err("Poll interval must be positive".into());
  }
  if !listener.local_addr().map_err(|e| e.to_string())?.ip().is_loopback() {
    return Err("Relay service listener must be loopback-only".into());
  }
  let service = Service::new(config)?;
  let connections = Arc::new(Semaphore::new(64));
  // Dropping the service future closes its client sockets as well as its
  // listener, so consumers actually reconnect after a service shutdown.
  let mut peers = tokio::task::JoinSet::new();
  loop {
    let accepted = tokio::select! {
      accepted = listener.accept() => accepted,
      _ = peers.join_next(), if !peers.is_empty() => continue,
    };
    let (mut stream, _) = accepted.map_err(|err| err.to_string())?;
    let Ok(permit) = connections.clone().try_acquire_owned() else {
      continue;
    };
    let service = service.clone();
    peers.spawn(async move {
      let _permit = permit;
      if let Err(message) = service.handle(&mut stream).await {
        let _ = tokio::time::timeout(
          Duration::from_secs(2),
          write_frame(&mut stream, &Frame::Error { message }),
        )
        .await;
      }
    });
  }
}

impl Service {
  pub fn new(config: RelayConfig) -> Result<Arc<Self>, String> {
    Self::with_index(config, None)
  }

  pub(crate) fn from_index(
    config: RelayConfig,
    index: Arc<tokn_session_index::SessionIndex>,
  ) -> Result<Arc<Self>, String> {
    Self::with_index(config, Some(index))
  }

  fn with_index(
    config: RelayConfig,
    index: Option<Arc<tokn_session_index::SessionIndex>>,
  ) -> Result<Arc<Self>, String> {
    if config.poll_interval.is_zero() {
      return Err("Poll interval must be positive".into());
    }
    let service = Arc::new(Self {
      index,
      config,
      sessions: Mutex::new(HashMap::new()),
      catalog: Mutex::new(None),
      metadata: Arc::new(PresentationCache::default()),
      wake: watch::channel(()).0,
    });
    let weak = Arc::downgrade(&service);
    if service.index.is_none() {
      tokio::spawn(async move {
        loop {
          tokio::time::sleep(Duration::from_millis(250)).await;
          let Some(service) = weak.upgrade() else {
            return;
          };
          let metadata = service.metadata.clone();
          drop(service);
          if tokio::task::spawn_blocking(move || metadata.step()).await.is_err() {
            return;
          }
        }
      });
    }
    Ok(service)
  }

  /// Live feed notifications only wake authoritative readers. Polling remains
  /// the recovery path for feed startup gaps, restarts, or omitted records.
  pub async fn invalidate(&self) {
    *self.catalog.lock().await = None;
    self.wake.send_replace(());
  }

  async fn catalog(&self) -> Result<(Arc<Vec<CatalogEntry>>, Vec<String>), String> {
    if let Some(index) = &self.index {
      let index = index.clone();
      let entries = tokio::task::spawn_blocking(move || crate::index_queries::snapshot_entries(&index))
        .await
        .map_err(|error| error.to_string())??;
      return Ok((Arc::new(entries), Vec::new()));
    }
    let mut cached = self.catalog.lock().await;
    if let Some((time, entries, warnings)) = cached.as_ref()
      && time.elapsed() < Duration::from_secs(2)
    {
      return Ok((Arc::new(self.metadata.decorate(entries)), warnings.clone()));
    }
    let roots = self.config.roots.clone();
    let metadata = self.metadata.clone();
    let (entries, warnings) = tokio::task::spawn_blocking(move || {
      let mut entries = Vec::new();
      let mut warnings = Vec::new();
      for root in roots {
        // Providers are optional installations. An absent resolved root is an
        // empty source; corrupt or inaccessible existing roots still report errors.
        if matches!(std::fs::metadata(&root.path), Err(error) if error.kind() == std::io::ErrorKind::NotFound) {
          continue;
        }
        let source = tokn_session_relay::providers::source(root.provider);
        match AgentClient::list_session_headers(source, Some(root.path)) {
          Ok(headers) => entries.extend(headers.into_iter().map(|header| CatalogEntry {
            key: serde_json::to_string(&(root.provider, &header.path, &header.id)).expect("header key is serializable"),
            provider: root.provider,
            header,
          })),
          Err(error) => warnings.push(format!("{:?}: {error}", root.provider)),
        }
      }
      entries.sort_by(|a, b| a.key.cmp(&b.key));
      entries.dedup_by(|a, b| a.key == b.key);
      metadata.reconcile(&entries);
      (entries, warnings)
    })
    .await
    .map_err(|err| err.to_string())?;
    let entries = Arc::new(entries);
    *cached = Some((std::time::Instant::now(), entries.clone(), warnings.clone()));
    Ok((Arc::new(self.metadata.decorate(&entries)), warnings))
  }

  async fn follow(&self, key: &str) -> Result<Arc<FollowedSession>, String> {
    let mut sessions = self.sessions.lock().await;
    if let Some(session) = sessions.get(key).and_then(Weak::upgrade) {
      return Ok(session);
    }
    sessions.retain(|_, value| value.strong_count() > 0);
    if sessions.len() >= 16 {
      return Err("Relay active-session limit reached; close an unused viewer session".into());
    }
    let entry = if let Some(index) = &self.index {
      let index = index.clone();
      let key = key.to_owned();
      tokio::task::spawn_blocking(move || crate::index_queries::snapshot_entry_for_key(&index, &key))
        .await
        .map_err(|e| e.to_string())??
    } else {
      let (catalog, _) = self.catalog().await?;
      catalog.iter().find(|entry| entry.key == key).cloned()
    }
    .ok_or(if self.index.is_some() {
      "Unknown session; refresh the index"
    } else {
      "Unknown Relay session; refresh the catalog"
    })?;
    let native = self.config.include_native;
    let root = self
      .config
      .roots
      .iter()
      .find(|root| root.provider == entry.provider && entry.header.path.starts_with(&root.path))
      .or_else(|| self.config.roots.iter().find(|root| root.provider == entry.provider))
      .ok_or("Relay provider is no longer configured")?
      .path
      .clone();
    let reader = tokio::task::spawn_blocking(move || SessionReader::new(entry, native, root))
      .await
      .map_err(|e| e.to_string())??;
    let (current, _) = watch::channel(Arc::new(reader.snapshot.clone()));
    let session = Arc::new(FollowedSession { current });
    sessions.insert(key.to_owned(), Arc::downgrade(&session));
    // The worker must not keep its own subscription alive. A weak handle also
    // avoids racing a last-client strong-count check against a new follow.
    let worker = Arc::downgrade(&session);
    let interval = self.config.poll_interval;
    let metadata = self.metadata.clone();
    let index = self.index.clone();
    let mut wake = self.wake.subscribe();
    tokio::spawn(async move {
      let mut reader = reader;
      loop {
        tokio::select! { _ = tokio::time::sleep(interval) => {}, result = wake.changed() => { if result.is_err() { break; } } }
        let Some(worker) = worker.upgrade() else {
          break;
        };
        let metadata = metadata.clone();
        let index = index.clone();
        let result = tokio::task::spawn_blocking(move || {
          let result = reader.poll().and_then(|changed| {
            if let Some(index) = &index {
              if let Some(entry) = crate::index_queries::snapshot_entry_for_key(index, &reader.snapshot.entry.key)? {
                let header = &mut reader.snapshot.entry.header;
                if header.title != entry.header.title || header.preview != entry.header.preview {
                  header.title = entry.header.title;
                  header.preview = entry.header.preview;
                  reader.snapshot.revision += 1;
                  return Ok(true);
                }
              }
              return Ok(changed);
            }
            // Refresh even when a metadata-only source row produced no events.
            metadata.refresh_followed(&reader.snapshot.entry);
            // OpenCode follows already load current presentation metadata.
            // JSONL follows need their separately cached names/previews.
            if matches!(reader.snapshot.entry.provider, Provider::Codex | Provider::Pi) {
              let before = reader.snapshot.entry.header.clone();
              metadata.apply(
                &reader.snapshot.entry.key,
                reader.snapshot.entry.provider,
                &mut reader.snapshot.entry.header,
              );
              if before != reader.snapshot.entry.header {
                reader.snapshot.revision += 1;
                return Ok(true);
              }
            }
            Ok(changed)
          });
          (reader, result)
        })
        .await;
        let (next, result) = match result {
          Ok(result) => result,
          Err(error) => {
            let mut failed = worker.current.borrow().as_ref().clone();
            failed.error = Some(format!("Relay session reader stopped: {error}"));
            worker.current.send_replace(Arc::new(failed));
            break;
          }
        };
        reader = next;
        match result {
          Ok(true) => {
            worker.current.send_replace(Arc::new(reader.snapshot.clone()));
          }
          Ok(false) => {}
          Err(error) => {
            reader.snapshot.error = Some(error);
            worker.current.send_replace(Arc::new(reader.snapshot.clone()));
            break;
          }
        }
      }
    });
    Ok(session)
  }

  pub async fn handle(
    &self,
    stream: &mut (impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send),
  ) -> Result<(), String> {
    let request: Request = tokio::time::timeout(Duration::from_secs(10), read_frame(stream))
      .await
      .map_err(|_| "Relay request timed out")??;
    if request.version != PROTOCOL_VERSION {
      return Err("Unsupported Relay protocol version".into());
    }
    let mut providers = Vec::new();
    for root in &self.config.roots {
      if !providers.contains(&root.provider) {
        providers.push(root.provider);
      }
    }
    send(
      stream,
      Frame::Hello {
        version: PROTOCOL_VERSION,
        native: self.config.include_native,
        providers,
      },
    )
    .await?;
    match request.action {
      Action::Catalog => {
        let (entries, warnings) = self.catalog().await?;
        for entry in entries.iter() {
          send(stream, Frame::Header { entry: entry.clone() }).await?;
        }
        send(stream, Frame::CatalogEnd { warnings }).await
      }
      Action::Follow { session_key } => {
        let session = self.follow(&session_key).await?;
        let mut changes = session.current.subscribe();
        let mut previous_generation = String::new();
        let mut previous_length = 0;
        loop {
          let snapshot = changes.borrow_and_update().clone();
          if let Some(error) = &snapshot.error {
            return Err(error.clone());
          }
          let reset = previous_generation != snapshot.generation;
          send(
            stream,
            Frame::Begin {
              generation: snapshot.generation.clone(),
              revision: snapshot.revision.to_string(),
              reset,
              header: snapshot.entry.header.clone(),
            },
          )
          .await?;
          for record in &snapshot.records[if reset { 0 } else { previous_length }..] {
            send(
              stream,
              Frame::Record {
                record: Box::new(record.as_ref().clone()),
              },
            )
            .await?;
          }
          send(
            stream,
            Frame::Commit {
              generation: snapshot.generation.clone(),
              revision: snapshot.revision.to_string(),
            },
          )
          .await?;
          previous_generation = snapshot.generation.clone();
          previous_length = snapshot.records.len();
          loop {
            tokio::select! {
              result = changes.changed() => { result.map_err(|_| "Relay session reader stopped")?; break; }
              _ = tokio::time::sleep(Duration::from_secs(2)) => send(stream, Frame::Heartbeat).await?,
            }
          }
        }
      }
    }
  }
}

async fn send(stream: &mut (impl tokio::io::AsyncWrite + Unpin), frame: Frame) -> Result<(), String> {
  tokio::time::timeout(Duration::from_secs(10), write_frame(stream, &frame))
    .await
    .map_err(|_| "Relay subscriber is too slow; reconnect for a fresh snapshot")?
}
