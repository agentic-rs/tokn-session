use super::*;
use crate::connector::{self, ConnectorConfig};
use axum::{
  Router,
  extract::{State, WebSocketUpgrade},
  routing::{get, post},
};
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};

async fn serve(app: Router) -> (url::Url, JoinHandle<()>) {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let url = format!("http://{}", listener.local_addr().unwrap()).parse().unwrap();
  let task = tokio::spawn(async move {
    axum::serve(listener, app).await.unwrap();
  });
  (url, task)
}

async fn wait_for(mut predicate: impl FnMut() -> bool) {
  tokio::time::timeout(Duration::from_secs(5), async {
    while !predicate() {
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
  })
  .await
  .expect("condition timed out");
}

async fn hub(tunnels: HubTunnels) -> (url::Url, JoinHandle<()>) {
  serve(
    Router::new()
      .route(
        protocol::TUNNEL_PATH,
        get(|State(tunnels): State<HubTunnels>, ws: WebSocketUpgrade| async move {
          ws.max_message_size(protocol::MAX_FRAME)
            .on_upgrade(move |socket| async move {
              tunnels.handle_socket(socket).await;
            })
        }),
      )
      .with_state(tunnels),
  )
  .await
}

struct StreamGuard(Arc<AtomicBool>);
impl Drop for StreamGuard {
  fn drop(&mut self) {
    self.0.store(false, Ordering::SeqCst);
  }
}

#[tokio::test]
async fn enrollment_streaming_cancellation_reconnect_and_revocation() {
  let directory = tempfile::tempdir().unwrap();
  let store = Store::open(directory.path().join("hub.sqlite")).unwrap();
  let tunnels = HubTunnels::new(store.clone());
  let (hub_url, hub_task) = hub(tunnels.clone()).await;
  let streaming = Arc::new(AtomicBool::new(false));
  let stream_state = streaming.clone();
  let local = Router::new()
    .route(
      "/api/v1/health",
      get(|| async { ([("content-type", "text/html")], "<script>bad()</script>") }),
    )
    .route("/api/v1/list_sessions", post(|| async { "x".repeat(2 * 1024 * 1024) }))
    .route(
      "/api/v1/events",
      get(move || {
        let streaming = stream_state.clone();
        async move {
          let stream = async_stream::stream! {
            let _guard = StreamGuard(streaming.clone());
            streaming.store(true, Ordering::SeqCst);
            yield Ok::<_, io::Error>(Bytes::from_static(b"event: ready\ndata: {}\n\n"));
            std::future::pending::<()>().await;
          };
          Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from_stream(stream))
            .unwrap()
        }
      }),
    );
  let (local_url, local_task) = serve(local).await;
  let mut config = ConnectorConfig {
    hub_url,
    local_url,
    key_file: directory.path().join("host.json"),
    name: "test host".into(),
    local_token: None,
    allow_control: false,
    insecure_loopback: true,
  };
  let stop = CancellationToken::new();
  let task = tokio::spawn(connector::run(config.clone(), stop.clone()));
  wait_for(|| !tunnels.pending().is_empty()).await;
  assert!(store.hosts().unwrap().is_empty());
  let pending = tunnels.pending().remove(0);
  let record = tunnels.approve(&pending.pairing_code).unwrap();
  assert!(
    tunnels.approve(&pending.pairing_code).is_err(),
    "pairing codes are single use"
  );
  wait_for(|| tunnels.online(&record.host_id)).await;

  let response = tunnels
    .proxy(&record.host_id, Method::GET, "/api/v1/health", Bytes::new())
    .await
    .ok()
    .unwrap();
  assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
  assert_eq!(response.headers()["x-content-type-options"], "nosniff");
  let body = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
  assert_eq!(&body[..], b"<script>bad()</script>");
  let response = tunnels
    .proxy(
      &record.host_id,
      Method::POST,
      "/api/v1/list_sessions",
      Bytes::from_static(b"{}"),
    )
    .await
    .ok()
    .unwrap();
  // More than one flow-control window must arrive intact, without spooling the
  // entire body in either process or starving heartbeat/cancellation traffic.
  let body = axum::body::to_bytes(response.into_body(), 3 * 1024 * 1024)
    .await
    .unwrap();
  assert_eq!(body.len(), 2 * 1024 * 1024);
  // A client that stops reading must not block requests on other streams.
  let stalled = tunnels
    .proxy(
      &record.host_id,
      Method::POST,
      "/api/v1/list_sessions",
      Bytes::from_static(b"{}"),
    )
    .await
    .ok()
    .unwrap();
  let other = tokio::time::timeout(
    Duration::from_secs(2),
    tunnels.proxy(&record.host_id, Method::GET, "/api/v1/health", Bytes::new()),
  )
  .await
  .unwrap()
  .ok()
  .unwrap();
  axum::body::to_bytes(other.into_body(), 1024).await.unwrap();
  drop(stalled);
  let denied = tunnels
    .proxy(
      &record.host_id,
      Method::POST,
      "/api/v1/submit_session_input",
      Bytes::new(),
    )
    .await
    .err()
    .unwrap();
  assert_eq!(denied.status, StatusCode::FORBIDDEN);

  let response = tunnels
    .proxy(&record.host_id, Method::GET, "/api/v1/events", Bytes::new())
    .await
    .ok()
    .unwrap();
  let mut body = response.into_body();
  let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
  assert!(String::from_utf8_lossy(&frame).contains("ready"));
  assert!(streaming.load(Ordering::SeqCst));
  drop(body);
  wait_for(|| !streaming.load(Ordering::SeqCst)).await;

  stop.cancel();
  task.await.unwrap().unwrap();
  wait_for(|| !tunnels.online(&record.host_id)).await;
  // The same host key reconnects without pairing, but changing its local flag
  // cannot exceed the access approved during the original enrollment.
  config.allow_control = true;
  let stop = CancellationToken::new();
  let task = tokio::spawn(connector::run(config, stop.clone()));
  wait_for(|| tunnels.online(&record.host_id)).await;
  assert!(tunnels.pending().is_empty());
  assert_eq!(tunnels.access(&record.host_id).as_deref(), Some("view"));
  let response = tunnels
    .proxy(&record.host_id, Method::GET, "/api/v1/events", Bytes::new())
    .await
    .ok()
    .unwrap();
  let mut body = response.into_body();
  body.frame().await.unwrap().unwrap();
  tunnels.revoke(&record.host_id).unwrap();
  assert!(!tunnels.online(&record.host_id));
  assert!(store.host(&record.host_id).unwrap().is_none());
  assert!(
    body.frame().await.unwrap().is_err(),
    "revocation terminates an established stream"
  );
  wait_for(|| !streaming.load(Ordering::SeqCst)).await;
  wait_for(|| !tunnels.pending().is_empty()).await;
  assert!(
    !tunnels.online(&record.host_id),
    "revocation requires a fresh owner approval"
  );
  stop.cancel();
  task.await.unwrap().unwrap();
  tunnels.shutdown();
  hub_task.abort();
  local_task.abort();
}

#[tokio::test]
async fn signatures_bind_fresh_challenge_identity_name_and_permission() {
  let directory = tempfile::tempdir().unwrap();
  let store = Store::open(directory.path().join("hub.sqlite")).unwrap();
  let tunnels = HubTunnels::new(store);
  let (mut endpoint, task) = hub(tunnels.clone()).await;
  endpoint.set_scheme("ws").unwrap();
  endpoint.set_path(protocol::TUNNEL_PATH);
  let key = SigningKey::generate(&mut OsRng);
  let public_key = protocol::encode(key.verifying_key().as_bytes());
  let mut previous_nonce = None;
  for attempt in 0..4 {
    let (mut socket, _) = connect_async(endpoint.as_str()).await.unwrap();
    let ClientMessage::Text(text) = socket.next().await.unwrap().unwrap() else {
      panic!("expected challenge")
    };
    let Frame::Challenge { nonce, .. } = serde_json::from_str(&text).unwrap() else {
      panic!("expected challenge")
    };
    let signed_nonce = if attempt == 1 {
      previous_nonce.as_ref().unwrap()
    } else {
      &nonce
    };
    let signing_key = if attempt == 0 {
      SigningKey::generate(&mut OsRng)
    } else {
      key.clone()
    };
    let signature = signing_key.sign(&protocol::proof(signed_nonce, &public_key, "Host", false));
    previous_nonce = Some(nonce);
    let frame = Frame::Authenticate {
      version: protocol::VERSION,
      public_key: public_key.clone(),
      name: "Host".into(),
      allow_control: attempt == 2,
      signature: protocol::encode(&signature.to_bytes()),
    };
    socket
      .send(ClientMessage::Text(serde_json::to_string(&frame).unwrap().into()))
      .await
      .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(2), socket.next())
      .await
      .unwrap();
    if attempt < 3 {
      assert!(
        !matches!(reply, Some(Ok(ClientMessage::Text(_)))),
        "invalid proof must not enter enrollment"
      );
      assert!(tunnels.pending().is_empty());
    } else {
      let Some(Ok(ClientMessage::Text(text))) = reply else {
        panic!("expected pairing")
      };
      assert!(matches!(
        serde_json::from_str::<Frame>(&text).unwrap(),
        Frame::Pending { .. }
      ));
      assert_eq!(tunnels.pending().len(), 1);
    }
  }
  tunnels.shutdown();
  task.abort();
}

type TestSocket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn signed_socket(endpoint: &url::Url, key: &SigningKey) -> TestSocket {
  let (mut socket, _) = connect_async(endpoint.as_str()).await.unwrap();
  let Some(Frame::Challenge { nonce, .. }) = next_frame(&mut socket).await else {
    panic!("expected challenge")
  };
  let public_key = protocol::encode(key.verifying_key().as_bytes());
  let signature = key.sign(&protocol::proof(&nonce, &public_key, "Host", false));
  let frame = Frame::Authenticate {
    version: protocol::VERSION,
    public_key,
    name: "Host".into(),
    allow_control: false,
    signature: protocol::encode(&signature.to_bytes()),
  };
  socket
    .send(ClientMessage::Text(serde_json::to_string(&frame).unwrap().into()))
    .await
    .unwrap();
  socket
}

async fn next_frame(socket: &mut TestSocket) -> Option<Frame> {
  tokio::time::timeout(Duration::from_secs(2), async {
    loop {
      match socket.next().await {
        Some(Ok(ClientMessage::Text(text))) => return Some(serde_json::from_str(&text).unwrap()),
        Some(Ok(ClientMessage::Ping(data))) => {
          let _ = socket.send(ClientMessage::Pong(data)).await;
        }
        Some(Ok(ClientMessage::Pong(_))) => {}
        _ => return None,
      }
    }
  })
  .await
  .expect("tunnel frame timed out")
}

#[tokio::test]
async fn pending_enrollments_cannot_exhaust_approved_reconnection_capacity() {
  let directory = tempfile::tempdir().unwrap();
  let store = Store::open(directory.path().join("hub.sqlite")).unwrap();
  let approved_key = SigningKey::generate(&mut OsRng);
  let host_id = protocol::host_id(approved_key.verifying_key().as_bytes());
  store
    .approve_host(HostRecord {
      host_id: host_id.clone(),
      name: "Approved".into(),
      public_key: protocol::encode(approved_key.verifying_key().as_bytes()),
      access: "view".into(),
    })
    .unwrap();
  let tunnels = HubTunnels::with_limits(store, 1, 2, 1);
  let (mut endpoint, task) = hub(tunnels.clone()).await;
  endpoint.set_scheme("ws").unwrap();
  endpoint.set_path(protocol::TUNNEL_PATH);

  let mut pending_sockets = Vec::new();
  for _ in 0..2 {
    let mut socket = signed_socket(&endpoint, &SigningKey::generate(&mut OsRng)).await;
    assert!(matches!(next_frame(&mut socket).await, Some(Frame::Pending { .. })));
    pending_sockets.push(socket);
  }
  assert_eq!(tunnels.pending().len(), 2);
  assert_eq!(tunnels.enrollments.available_permits(), 0);
  assert_eq!(tunnels.handshakes.available_permits(), 1);
  assert_eq!(tunnels.approved.available_permits(), 1);
  let mut overflow = signed_socket(&endpoint, &SigningKey::generate(&mut OsRng)).await;
  assert!(
    next_frame(&mut overflow).await.is_none(),
    "unknown hosts cannot exceed the pending budget"
  );

  let mut approved = signed_socket(&endpoint, &approved_key).await;
  assert!(matches!(next_frame(&mut approved).await, Some(Frame::Ready { .. })));
  assert!(tunnels.online(&host_id));
  assert_eq!(tunnels.approved.available_permits(), 0);
  // All long-lived budgets are full. An approved identity can still replace
  // its own connection, and closing the old socket must not release its slot.
  let mut replacement = signed_socket(&endpoint, &approved_key).await;
  assert!(matches!(next_frame(&mut replacement).await, Some(Frame::Ready { .. })));
  assert!(next_frame(&mut approved).await.is_none());
  assert!(tunnels.online(&host_id));
  assert_eq!(tunnels.approved.available_permits(), 0);
  assert_eq!(tunnels.pending().len(), 2);
  drop(replacement);
  wait_for(|| !tunnels.online(&host_id) && tunnels.approved.available_permits() == 1).await;
  drop(pending_sockets);
  wait_for(|| tunnels.enrollments.available_permits() == 2).await;
  tunnels.shutdown();
  task.abort();
}

#[tokio::test]
async fn accepted_agent_input_is_not_replayed_after_tunnel_reconnect() {
  let directory = tempfile::tempdir().unwrap();
  let store = Store::open(directory.path().join("hub.sqlite")).unwrap();
  let tunnels = HubTunnels::new(store);
  let (hub_url, hub_task) = hub(tunnels.clone()).await;
  let accepted_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
  let count = accepted_count.clone();
  let (accepted, mut acceptances) = mpsc::channel(8);
  let local = Router::new().route("/api/v1/health", get(|| async { "{}" })).route(
    "/api/v1/submit_session_input",
    post(move || {
      let count = count.clone();
      let accepted = accepted.clone();
      async move {
        count.fetch_add(1, Ordering::SeqCst);
        let _ = accepted.send(()).await;
        // The agent has accepted the input, but the caller never receives a
        // response establishing whether it was delivered.
        std::future::pending::<()>().await;
        "{}"
      }
    }),
  );
  let (local_url, local_task) = serve(local).await;
  let config = ConnectorConfig {
    hub_url,
    local_url,
    key_file: directory.path().join("host.json"),
    name: "Input host".into(),
    local_token: None,
    allow_control: true,
    insecure_loopback: true,
  };
  let stop = CancellationToken::new();
  let connector_task = tokio::spawn(connector::run(config, stop.clone()));
  wait_for(|| !tunnels.pending().is_empty()).await;
  let pending = tunnels.pending().remove(0);
  let record = tunnels.approve(&pending.pairing_code).unwrap();
  wait_for(|| tunnels.online(&record.host_id)).await;
  let old_connection = tunnels.inner.lock().unwrap().online[&record.host_id].clone();
  let proxy = tunnels.clone();
  let host_id = record.host_id.clone();
  let request = tokio::spawn(async move {
    proxy
      .proxy(
        &host_id,
        Method::POST,
        "/api/v1/submit_session_input",
        Bytes::from_static(br#"{"request":{"text":"execute once"}}"#),
      )
      .await
  });
  tokio::time::timeout(Duration::from_secs(2), acceptances.recv())
    .await
    .unwrap()
    .unwrap();
  assert_eq!(accepted_count.load(Ordering::SeqCst), 1);
  old_connection.shutdown.cancel();
  let failure = tokio::time::timeout(Duration::from_secs(2), request)
    .await
    .unwrap()
    .unwrap()
    .err()
    .unwrap();
  assert_eq!(failure.status, StatusCode::BAD_GATEWAY);
  assert!(failure.message.contains("never replayed"));

  // Exercise the production reconnect loop with the same approved identity.
  // A fresh successful request proves its new tunnel is usable.
  wait_for(|| tunnels.online(&record.host_id)).await;
  assert!(tunnels.pending().is_empty());
  let response = tunnels
    .proxy(&record.host_id, Method::GET, "/api/v1/health", Bytes::new())
    .await
    .ok()
    .unwrap();
  assert_eq!(axum::body::to_bytes(response.into_body(), 1024).await.unwrap(), "{}");
  assert!(
    tokio::time::timeout(Duration::from_millis(200), acceptances.recv())
      .await
      .is_err(),
    "uncertain input must not be retried by HTTP or by tunnel reconnect"
  );
  assert_eq!(accepted_count.load(Ordering::SeqCst), 1);
  stop.cancel();
  connector_task.await.unwrap().unwrap();
  tunnels.shutdown();
  hub_task.abort();
  local_task.abort();
}
