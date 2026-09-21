//! Host-owned trust boundary for encrypted requests. The Hub is only a carrier.
use super::{ConnectorConfig, SecureHostConfig, enqueue};
use crate::{
  protocol::{self, Frame},
  secure::{InnerMessage, NoiseIdentity, NoiseResponder, SecureChannel, SignedGrant},
};
use futures_util::StreamExt;
use std::{
  collections::HashSet,
  fs::OpenOptions,
  io::Read,
  path::Path,
  sync::Mutex,
  time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Semaphore, mpsc};

pub(super) struct Host {
  identity: NoiseIdentity,
  config: SecureHostConfig,
}

impl Host {
  pub(super) fn load(config: &SecureHostConfig) -> Result<Self, String> {
    let bytes: [u8; 32] = protocol::decode(&config.owner_public_key, 32)?
      .try_into()
      .map_err(|_| "Invalid owner public key")?;
    ed25519_dalek::VerifyingKey::from_bytes(&bytes).map_err(|_| "Invalid owner public key")?;
    let host = Self {
      identity: NoiseIdentity::load_or_create(&config.noise_key_file)?,
      config: config.clone(),
    };
    host.revocations()?;
    Ok(host)
  }

  pub(super) fn public_key(&self) -> String {
    self.identity.public_key()
  }

  fn revocations(&self) -> Result<HashSet<String>, String> {
    self
      .config
      .revocations_file
      .as_deref()
      .map(read_revocations)
      .transpose()
      .map(|value| value.unwrap_or_default())
  }

  fn verify(&self, grant: &SignedGrant, host_id: &str, recipient: &str) -> Result<(), String> {
    let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map_err(|_| "Invalid host clock")?
      .as_secs();
    grant.verify(
      &self.config.owner_public_key,
      host_id,
      &self.public_key(),
      recipient,
      now,
      &self.revocations()?,
    )
  }
}

fn read_revocations(path: &Path) -> Result<HashSet<String>, String> {
  let mut options = OpenOptions::new();
  options.read(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
  }
  #[cfg(not(unix))]
  if std::fs::symlink_metadata(path)
    .map_err(|_| "Cannot inspect grant revocations")?
    .file_type()
    .is_symlink()
  {
    return Err("Grant revocations must not be a symlink".into());
  }
  let file = options
    .open(path)
    .map_err(|_| "Cannot read configured grant revocations")?;
  let metadata = file.metadata().map_err(|_| "Cannot inspect grant revocations")?;
  if !metadata.is_file() || metadata.len() > 1024 * 1024 {
    return Err("Grant revocations must be a regular file of at most 1 MiB".into());
  }
  let mut bytes = Vec::new();
  file
    .take(1024 * 1024 + 1)
    .read_to_end(&mut bytes)
    .map_err(|_| "Cannot read grant revocations")?;
  if bytes.len() > 1024 * 1024 {
    return Err("Grant revocations exceed 1 MiB".into());
  }
  let ids: Vec<String> = serde_json::from_slice(&bytes).map_err(|_| "Invalid grant revocations JSON")?;
  if ids
    .iter()
    .any(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
  {
    return Err("Invalid revoked grant identifier".into());
  }
  Ok(ids.into_iter().collect())
}

async fn receive(incoming: &mut mpsc::Receiver<Vec<u8>>) -> Result<Vec<u8>, String> {
  tokio::time::timeout(Duration::from_secs(10), incoming.recv())
    .await
    .map_err(|_| "Encrypted request timed out")?
    .ok_or_else(|| "Encrypted channel closed".into())
}

pub(super) async fn run(
  host: &Host,
  host_id: &str,
  config: &ConnectorConfig,
  client: &reqwest::Client,
  outgoing: &mpsc::Sender<Frame>,
  channel_id: u64,
  mut incoming: mpsc::Receiver<Vec<u8>>,
) -> Result<(), String> {
  let first = receive(&mut incoming).await?;
  let (reply, mut channel) = NoiseResponder::new(&host.identity)?.accept(&first)?;
  enqueue(
    outgoing,
    Frame::SecureData {
      channel_id,
      data: protocol::encode(&reply),
    },
  )
  .await?;
  let header = channel.decrypt(&receive(&mut incoming).await?)?;
  let InnerMessage::Request { method, path, grant } = header else {
    return Err("Expected encrypted request".into());
  };
  let recipient = channel.remote_public_key().to_owned();
  host.verify(&grant, host_id, &recipient)?;
  let body = tokio::time::timeout(Duration::from_secs(10), async {
    let mut body = Vec::new();
    let mut records = 0;
    loop {
      match channel.decrypt(&receive(&mut incoming).await?)? {
        InnerMessage::RequestBody { data } => {
          records += 1;
          if records > protocol::MAX_BODY.div_ceil(protocol::CHUNK_SIZE) {
            return Err("Too many request body records".into());
          }
          let bytes = protocol::decode(&data, protocol::CHUNK_SIZE)?;
          if body.len() + bytes.len() > protocol::MAX_BODY {
            return Err("Encrypted request body is too large".into());
          }
          body.extend_from_slice(&bytes);
        }
        InnerMessage::RequestEnd {} => return Ok::<_, String>(body),
        _ => return Err("Expected encrypted request body".into()),
      }
    }
  })
  .await
  .map_err(|_| "Encrypted request body timed out")??;
  host.verify(&grant, host_id, &recipient)?;
  let channel = Mutex::new(channel);
  let window = Semaphore::new(protocol::RESPONSE_WINDOW);
  let result = {
    let response = forward(
      config, client, &grant, &method, &path, body, outgoing, channel_id, &channel, &window,
    );
    let timeout = tokio::time::sleep(Duration::from_secs(120));
    tokio::pin!(response, timeout);
    let mut policy_check = tokio::time::interval(Duration::from_secs(1));
    loop {
      tokio::select! {
        biased;
        _ = policy_check.tick() => {
          if let Err(error) = host.verify(&grant, host_id, &recipient) { break Err(error); }
        }
        _ = &mut timeout, if path != "/api/v1/events" => break Err("Encrypted request timed out; delivery may be uncertain and is never retried".into()),
        record = incoming.recv() => {
          let Some(record) = record else { break Err("Encrypted channel closed".into()) };
          let message = channel.lock().unwrap().decrypt(&record);
          match message {
            Ok(InnerMessage::Window { credits }) if credits > 0 && credits <= protocol::RESPONSE_WINDOW && window.available_permits() + credits <= protocol::RESPONSE_WINDOW => window.add_permits(credits),
            _ => break Err("Invalid encrypted response window".into()),
          }
        }
        result = &mut response => break result,
      }
    }
  };
  if let Err(message) = &result {
    let _ = send_inner(
      outgoing,
      channel_id,
      &channel,
      &InnerMessage::Error {
        message: message.clone(),
      },
    )
    .await;
  } else {
    // End may be queued behind the final chunk at the recipient. Keep the
    // receive side alive while it acknowledges that chunk and reads End, rather
    // than racing its final credit with an abrupt relay socket close.
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
      while let Some(record) = incoming.recv().await {
        match channel.lock().unwrap().decrypt(&record) {
          Ok(InnerMessage::Window { credits })
            if credits > 0
              && credits <= protocol::RESPONSE_WINDOW
              && window.available_permits() + credits <= protocol::RESPONSE_WINDOW =>
          {
            window.add_permits(credits)
          }
          _ => break,
        }
      }
    })
    .await;
  }
  result
}

async fn send_inner(
  outgoing: &mpsc::Sender<Frame>,
  channel_id: u64,
  channel: &Mutex<SecureChannel>,
  message: &InnerMessage,
) -> Result<(), String> {
  let bytes = channel.lock().unwrap().encrypt(message)?;
  enqueue(
    outgoing,
    Frame::SecureData {
      channel_id,
      data: protocol::encode(&bytes),
    },
  )
  .await
}

#[allow(clippy::too_many_arguments)]
async fn forward(
  config: &ConnectorConfig,
  client: &reqwest::Client,
  grant: &SignedGrant,
  method: &str,
  path: &str,
  body: Vec<u8>,
  outgoing: &mpsc::Sender<Frame>,
  channel_id: u64,
  channel: &Mutex<SecureChannel>,
  window: &Semaphore,
) -> Result<(), String> {
  let response = crate::secure_scope::forward(config, client, &grant.grant, method, path, body).await?;
  if response.status().is_redirection() {
    return Err("Local API redirects are forbidden".into());
  }
  send_inner(
    outgoing,
    channel_id,
    channel,
    &InnerMessage::Response {
      status: response.status().as_u16(),
      content_type: Some(
        if path == "/api/v1/events" {
          "text/event-stream"
        } else {
          "application/json"
        }
        .into(),
      ),
    },
  )
  .await?;
  let mut stream = response.bytes_stream();
  let mut total = 0usize;
  while let Some(chunk) = stream.next().await {
    let chunk = chunk.map_err(|_| "Local API response stream failed")?;
    total = total.saturating_add(chunk.len());
    if path != "/api/v1/events" && total > 128 * 1024 * 1024 {
      return Err("Local API response exceeds 128 MiB".into());
    }
    for part in chunk.chunks(protocol::CHUNK_SIZE) {
      // Only the authenticated recipient can return these encrypted credits.
      // The Hub cannot manufacture acknowledgements to make us spool plaintext.
      window
        .acquire()
        .await
        .map_err(|_| "Encrypted response cancelled")?
        .forget();
      send_inner(
        outgoing,
        channel_id,
        channel,
        &InnerMessage::Chunk {
          data: protocol::encode(part),
        },
      )
      .await?;
    }
  }
  send_inner(outgoing, channel_id, channel, &InnerMessage::End {}).await
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;

  #[test]
  fn revocations_are_reloaded_and_invalid_files_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("revoked.json");
    assert!(read_revocations(&file).is_err());
    fs::write(&file, "[]").unwrap();
    assert!(read_revocations(&file).unwrap().is_empty());
    fs::write(&file, r#"["grant_1"]"#).unwrap();
    assert!(read_revocations(&file).unwrap().contains("grant_1"));
    fs::write(&file, "{}").unwrap();
    assert!(read_revocations(&file).is_err());
    #[cfg(unix)]
    {
      let link = directory.path().join("link.json");
      std::os::unix::fs::symlink(&file, &link).unwrap();
      assert!(read_revocations(&link).is_err());
      let fifo = directory.path().join("fifo");
      let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
      assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
      assert!(
        read_revocations(&fifo).is_err(),
        "a FIFO must fail without waiting for a writer"
      );
    }
  }
}
