use std::{
  collections::HashMap,
  sync::{Arc, Weak},
  time::Duration,
};

use tokio::{
  net::{TcpListener, TcpStream},
  sync::{Mutex, Semaphore, watch},
};
use tokn_session_client::{AgentClient, Source};
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

struct Service {
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
  let service = Arc::new(Service {
    config,
    sessions: Mutex::new(HashMap::new()),
    catalog: Mutex::new(None),
    metadata: Arc::new(PresentationCache::default()),
  });
  let connections = Arc::new(Semaphore::new(64));
  // Dropping the service future closes its client sockets as well as its
  // listener, so consumers actually reconnect after a service shutdown.
  let mut peers = tokio::task::JoinSet::new();
  let mut background = tokio::task::JoinSet::new();
  let metadata = service.metadata.clone();
  background.spawn(async move {
    loop {
      tokio::time::sleep(Duration::from_millis(250)).await;
      let metadata = metadata.clone();
      if tokio::task::spawn_blocking(move || metadata.step()).await.is_err() {
        break;
      }
    }
  });
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
  async fn catalog(&self) -> Result<(Arc<Vec<CatalogEntry>>, Vec<String>), String> {
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
        let source = match root.provider {
          Provider::Codex => Source::Codex,
          Provider::Pi => Source::Pi,
          Provider::OpenCode => Source::OpenCode,
          _ => {
            warnings.push(format!("Unsupported relay provider: {:?}", root.provider));
            continue;
          }
        };
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
    let (catalog, _) = self.catalog().await?;
    let entry = catalog
      .iter()
      .find(|entry| entry.key == key)
      .cloned()
      .ok_or("Unknown Relay session; refresh the catalog")?;
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
    tokio::spawn(async move {
      let mut reader = reader;
      loop {
        tokio::time::sleep(interval).await;
        let Some(worker) = worker.upgrade() else {
          break;
        };
        let metadata = metadata.clone();
        let result = tokio::task::spawn_blocking(move || {
          let result = reader.poll().map(|changed| {
            // Refresh even when a metadata-only source row produced no events.
            metadata.refresh_followed(&reader.snapshot.entry);
            // OpenCode follows already load current presentation metadata.
            // JSONL follows need their separately cached names/previews.
            if reader.snapshot.entry.provider != Provider::OpenCode {
              let before = reader.snapshot.entry.header.clone();
              metadata.apply(
                &reader.snapshot.entry.key,
                reader.snapshot.entry.provider,
                &mut reader.snapshot.entry.header,
              );
              if before != reader.snapshot.entry.header {
                reader.snapshot.revision += 1;
                return true;
              }
            }
            changed
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

  async fn handle(&self, stream: &mut TcpStream) -> Result<(), String> {
    let request: Request = tokio::time::timeout(Duration::from_secs(10), read_frame(stream))
      .await
      .map_err(|_| "Relay request timed out")??;
    if request.version != PROTOCOL_VERSION {
      return Err("Unsupported Relay protocol version".into());
    }
    let providers = self.config.roots.iter().map(|root| root.provider).collect();
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

async fn send(stream: &mut TcpStream, frame: Frame) -> Result<(), String> {
  tokio::time::timeout(Duration::from_secs(10), write_frame(stream, &frame))
    .await
    .map_err(|_| "Relay subscriber is too slow; reconnect for a fresh snapshot")?
}
