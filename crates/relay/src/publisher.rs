use zeromq::{PubSocket, Socket, SocketSend, ZmqMessage};

use crate::RelayEvent;

pub struct ZmqPublisher {
  socket: PubSocket,
}

impl ZmqPublisher {
  pub async fn bind(endpoint: &str) -> Result<Self, String> {
    let mut socket = PubSocket::new();
    socket
      .bind(endpoint)
      .await
      .map_err(|err| format!("failed to bind ZeroMQ publisher to {endpoint}: {err}"))?;
    Ok(Self { socket })
  }

  pub async fn publish(&mut self, event: &RelayEvent) -> Result<(), String> {
    let payload = serde_json::to_vec(&event.event).map_err(|err| format!("failed to serialize relay event: {err}"))?;
    let mut message = ZmqMessage::from(event.topic.as_str());
    message.push_back(payload.into());
    self
      .socket
      .send(message)
      .await
      .map_err(|err| format!("failed to publish relay event: {err}"))
  }
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;
  use std::time::Duration;

  use tempfile::TempDir;
  use tokn_session_core::{AgentEvent, MessageEvent, Phase, Provider, Role};
  use zeromq::{Socket, SocketRecv, SubSocket};

  use super::ZmqPublisher;
  use crate::RelayEvent;

  #[tokio::test]
  async fn publishes_topic_and_json_as_multipart_message() {
    let socket_dir = TempDir::new().unwrap();
    let endpoint = format!("ipc://{}", socket_dir.path().join("relay.sock").display());
    let mut publisher = ZmqPublisher::bind(&endpoint).await.unwrap();
    let mut subscriber = SubSocket::new();
    subscriber.connect(&endpoint).await.unwrap();
    subscriber.subscribe("pi.").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let event = AgentEvent::Message(MessageEvent {
      provider: Provider::Pi,
      session_id: Some("session-1".to_string()),
      message_id: Some("message-1".to_string()),
      parent_id: None,
      role: Role::Assistant,
      phase: Phase::Finished,
      text: "done".to_string(),
      timestamp: None,
    });
    publisher
      .publish(&RelayEvent {
        path: PathBuf::from("session.jsonl"),
        topic: "pi.session-1".to_string(),
        event,
      })
      .await
      .unwrap();

    let message = tokio::time::timeout(Duration::from_secs(2), subscriber.recv())
      .await
      .expect("subscriber timed out")
      .unwrap();
    assert_eq!(message.len(), 2);
    assert_eq!(message.get(0).unwrap().as_ref(), b"pi.session-1");
    let payload: serde_json::Value = serde_json::from_slice(message.get(1).unwrap()).unwrap();
    assert_eq!(payload["type"], "message");
    assert_eq!(payload["session_id"], "session-1");
    assert_eq!(payload["text"], "done");
  }
}
