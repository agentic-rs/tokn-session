use std::{sync::Arc, time::Duration};

use tokio::net::TcpStream;
use tokn_session_core::{LoadedSession, Provider, SessionHistoryStatus, SessionRef};

use crate::{RelayRecord, service_protocol::*};

pub struct RelayCatalog {
  pub entries: Vec<CatalogEntry>,
  pub providers: Vec<Provider>,
  pub native: bool,
  pub warnings: Vec<String>,
}

async fn connect(endpoint: &str, action: Action) -> Result<(TcpStream, Vec<Provider>, bool), String> {
  let mut stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(local_endpoint(endpoint)?))
    .await
    .map_err(|_| "Relay connection timed out")?
    .map_err(|e| e.to_string())?;
  write_frame(
    &mut stream,
    &Request {
      version: PROTOCOL_VERSION,
      action,
    },
  )
  .await?;
  match receive(&mut stream).await? {
    Frame::Hello {
      version: PROTOCOL_VERSION,
      native,
      providers,
    } => Ok((stream, providers, native)),
    _ => Err("Unsupported Relay handshake".into()),
  }
}

pub async fn load_catalog(endpoint: &str) -> Result<RelayCatalog, String> {
  let (mut stream, providers, native) = connect(endpoint, Action::Catalog).await?;
  let mut entries = Vec::new();
  let mut bytes = 0;
  loop {
    match receive(&mut stream).await? {
      Frame::Header { entry } => {
        bytes += serde_json::to_vec(&entry).map_err(|e| e.to_string())?.len();
        if bytes > MAX_SNAPSHOT_BYTES {
          return Err("Relay catalog exceeds memory limit".into());
        }
        entries.push(entry);
      }
      Frame::CatalogEnd { warnings } => {
        return Ok(RelayCatalog {
          entries,
          providers,
          native,
          warnings,
        });
      }
      _ => return Err("Unexpected Relay catalog frame".into()),
    }
  }
}

pub struct RelaySubscription {
  stream: TcpStream,
  generation: String,
  revision: u64,
  records: Vec<RelayRecord>,
  bytes: usize,
}

#[derive(Debug)]
pub struct SessionSnapshot {
  pub generation: String,
  pub revision: String,
  pub loaded: LoadedSession,
  /// Native record for each normalized event, used by the viewer Inspector.
  pub native: Vec<Option<Arc<serde_json::Value>>>,
}

impl RelaySubscription {
  pub async fn connect(endpoint: &str, session_key: &str) -> Result<Self, String> {
    let (stream, _, _) = connect(
      endpoint,
      Action::Follow {
        session_key: session_key.into(),
      },
    )
    .await?;
    Ok(Self {
      stream,
      generation: String::new(),
      revision: 0,
      records: Vec::new(),
      bytes: 0,
    })
  }

  /// No partial snapshot escapes this method. Interrupted transactions leave
  /// the caller's last committed snapshot intact and require reconnection.
  pub async fn next_snapshot(&mut self) -> Result<SessionSnapshot, String> {
    let (generation, revision, reset, header) = loop {
      match receive(&mut self.stream).await? {
        Frame::Begin {
          generation,
          revision,
          reset,
          header,
        } => break (generation, revision, reset, header),
        Frame::Heartbeat => continue,
        _ => return Err("Expected Relay snapshot boundary".into()),
      }
    };
    let sequence: u64 = revision.parse().map_err(|_| "Invalid Relay revision")?;
    if !reset && (generation != self.generation || sequence <= self.revision) {
      return Err("Relay cursor mismatch; resnapshot required".into());
    }
    let mut pending = Vec::new();
    let mut bytes = if reset { 0 } else { self.bytes };
    loop {
      match receive(&mut self.stream).await? {
        Frame::Record { record } => {
          if record.path != header.path || record.session.session_id != header.id {
            return Err("Relay snapshot contains a record outside its session".into());
          }
          bytes += serde_json::to_vec(&record).map_err(|e| e.to_string())?.len();
          if bytes > MAX_SNAPSHOT_BYTES {
            return Err("Relay snapshot exceeds memory limit".into());
          }
          pending.push(*record);
        }
        Frame::Commit {
          generation: end_generation,
          revision: end_revision,
        } if end_generation == generation && end_revision == revision => break,
        _ => return Err("Invalid Relay snapshot transaction".into()),
      }
    }
    if reset {
      self.records.clear();
    }
    self.records.extend(pending);
    self.bytes = bytes;
    self.generation = generation.clone();
    self.revision = sequence;
    let mut events = Vec::new();
    let mut native = Vec::new();
    for record in &self.records {
      let redacted = record.record.events.iter().any(|event| {
        matches!(event,
        tokn_session_core::AgentEvent::Reasoning(reasoning) if reasoning.redacted == Some(true))
      });
      let source = if redacted {
        None
      } else {
        record.record.native.clone().map(Arc::new)
      };
      for event in &record.record.events {
        events.push(event.clone());
        native.push(source.clone());
      }
    }
    let message_count = events
      .iter()
      .filter(|e| matches!(e, tokn_session_core::AgentEvent::Message(_)))
      .count();
    Ok(SessionSnapshot {
      generation,
      revision,
      native,
      loaded: LoadedSession {
        reference: SessionRef {
          id: header.id,
          parent_session_id: header.parent_session_id,
          agent_path: header.agent_path,
          agent_nickname: header.agent_nickname,
          agent_role: header.agent_role,
          title: header.title,
          preview: header.preview,
          path: header.path,
          cwd: header.cwd,
          timestamp: header.timestamp,
          message_count,
        },
        events,
        history_status: SessionHistoryStatus::Complete,
      },
    })
  }
}

async fn receive(stream: &mut TcpStream) -> Result<Frame, String> {
  let frame = tokio::time::timeout(Duration::from_secs(10), read_frame(stream))
    .await
    .map_err(|_| "Relay response timed out")??;
  if let Frame::Error { message } = frame {
    Err(message)
  } else {
    Ok(frame)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[tokio::test]
  async fn redacted_sibling_native_is_withheld_and_incomplete_reset_does_not_commit() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("tcp://{}", listener.local_addr().unwrap());
    let header: tokn_session_core::SessionHeader =
      serde_json::from_value(json!({ "id": "fixture", "path": "/tmp/fixture.jsonl" })).unwrap();
    let record: RelayRecord = serde_json::from_value(json!({
      "path": "/tmp/fixture.jsonl", "topic": "pi.fixture", "operation": "upsert", "record_id": "jsonl:0",
      "session": { "provider": "pi", "session_id": "fixture" }, "native": { "private": "redacted source" },
      "events": [
        { "type": "reasoning", "provider": "pi", "phase": "finished", "redacted": true },
        { "type": "session_started", "provider": "pi", "session_id": "fixture" }
      ]
    }))
    .unwrap();
    let server = tokio::spawn(async move {
      let (mut stream, _) = listener.accept().await.unwrap();
      let _: Request = read_frame(&mut stream).await.unwrap();
      for frame in [
        Frame::Hello {
          version: PROTOCOL_VERSION,
          providers: vec![Provider::Pi],
          native: true,
        },
        Frame::Begin {
          generation: "first".into(),
          revision: "1".into(),
          reset: true,
          header: header.clone(),
        },
        Frame::Record {
          record: Box::new(record.clone()),
        },
        Frame::Commit {
          generation: "first".into(),
          revision: "1".into(),
        },
        Frame::Begin {
          generation: "replacement".into(),
          revision: "2".into(),
          reset: true,
          header,
        },
        Frame::Record {
          record: Box::new(record),
        },
      ] {
        write_frame(&mut stream, &frame).await.unwrap();
      }
      // Drop the stream mid-transaction, before commit.
    });
    let mut subscription = RelaySubscription::connect(&endpoint, "fixture").await.unwrap();
    let first = subscription.next_snapshot().await.unwrap();
    assert_eq!(first.loaded.events.len(), 2);
    assert!(first.native.iter().all(Option::is_none));
    let bytes = subscription.bytes;
    assert!(subscription.next_snapshot().await.is_err());
    assert_eq!(subscription.generation, "first");
    assert_eq!(subscription.revision, 1);
    assert_eq!(subscription.records.len(), 1);
    assert_eq!(subscription.bytes, bytes);
    server.await.unwrap();
  }
}
