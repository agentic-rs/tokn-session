use std::{
  path::PathBuf,
  sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
  },
  time::SystemTime,
};

use tokn_session_core::Provider;
use tokn_session_opencode::OpenCodeSessionSource;

use crate::{RecordOperation, RelayRecord, SessionContext, service_protocol::CatalogEntry, tailer::FileState};

#[derive(Clone)]
pub(crate) struct Snapshot {
  pub generation: String,
  pub revision: u64,
  pub records: Vec<Arc<RelayRecord>>,
  pub entry: CatalogEntry,
  pub error: Option<String>,
}

fn generation() -> String {
  static COUNTER: AtomicU64 = AtomicU64::new(0);
  format!(
    "{}-{}-{}",
    std::process::id(),
    SystemTime::now()
      .duration_since(SystemTime::UNIX_EPOCH)
      .unwrap_or_default()
      .as_nanos(),
    COUNTER.fetch_add(1, Ordering::Relaxed)
  )
}

pub(crate) struct SessionReader {
  file: Option<FileState>,
  database: Option<OpenCodeSessionSource>,
  native: bool,
  root: PathBuf,
  bytes: usize,
  version: Vec<Option<FileVersion>>,
  pub snapshot: Snapshot,
}

impl SessionReader {
  pub fn new(entry: CatalogEntry, native: bool, root: PathBuf) -> Result<Self, String> {
    let mut reader = Self {
      file: if matches!(entry.provider, Provider::Codex | Provider::Pi) {
        Some(FileState::for_snapshot(
          entry.header.path.clone(),
          entry.provider,
          native,
          &root,
        )?)
      } else {
        None
      },
      database: matches!(entry.provider, Provider::OpenCode)
        .then(|| OpenCodeSessionSource::new(Some(entry.header.path.clone()))),
      native,
      root,
      bytes: 0,
      version: Vec::new(),
      snapshot: Snapshot {
        generation: generation(),
        revision: 0,
        records: Vec::new(),
        entry,
        error: None,
      },
    };
    reader.poll()?;
    Ok(reader)
  }

  pub fn poll(&mut self) -> Result<bool, String> {
    let path = &self.snapshot.entry.header.path;
    let version = versions(path, self.database.is_some());
    if self.file.is_some()
      && version[0]
        .as_ref()
        .is_some_and(|v| v.length > crate::service_protocol::MAX_SNAPSHOT_BYTES as u64)
    {
      return Err("Relay session exceeds the snapshot size limit".into());
    }
    if version == self.version && self.snapshot.error.is_none() {
      return Ok(false);
    }
    let (records, reset) = if let Some(file) = &mut self.file {
      // Same-length rewrites need a fresh reader; truncation/replacement is
      // also detected inside FileState before its next append read.
      let same_length_edit =
        !self.version.is_empty() && self.version[0].as_ref().map(|v| v.length) == version[0].as_ref().map(|v| v.length);
      if same_length_edit {
        *file = FileState::for_snapshot(path.clone(), self.snapshot.entry.provider, self.native, &self.root)?;
      }
      let (update, reset) = file.follow_snapshot()?;
      if !update.warnings.is_empty() {
        return Err(update.warnings.join("; "));
      }
      (update.records, reset || same_length_edit)
    } else if let Some(database) = &self.database {
      let loaded = database.load_session_records_exact(&self.snapshot.entry.header.id, self.native)?;
      let context = SessionContext::from_session_ref(&loaded.reference);
      let records = loaded
        .records
        .into_iter()
        .map(|record| RelayRecord {
          path: path.clone(),
          topic: format!("opencode.{}", context.session_id),
          session: context.clone(),
          operation: RecordOperation::Upsert,
          record,
        })
        .collect();
      (records, true)
    } else {
      return Err("Provider does not support snapshot/follow".into());
    };
    if records
      .iter()
      .any(|record| record.session.session_id != self.snapshot.entry.header.id)
    {
      return Err("Relay session identity changed; refresh the catalog".into());
    }
    // Charge only new records. Appends never serialize historical records.
    let mut bytes = if reset { 0 } else { self.bytes };
    for record in &records {
      bytes = bytes.saturating_add(serde_json::to_vec(record).map_err(|e| e.to_string())?.len());
      if bytes > crate::service_protocol::MAX_SNAPSHOT_BYTES {
        return Err("Relay session exceeds the snapshot memory limit".into());
      }
    }
    self.version = version;
    if records.is_empty() && !reset {
      return Ok(false);
    }
    if reset {
      self.snapshot.generation = generation();
      self.snapshot.records.clear();
    }
    self.snapshot.records.extend(records.into_iter().map(Arc::new));
    self.bytes = bytes;
    self.snapshot.revision += 1;
    self.snapshot.error = None;
    Ok(true)
  }
}

#[derive(PartialEq, Eq)]
struct FileVersion {
  length: u64,
  modified: SystemTime,
  #[cfg(unix)]
  identity: (u64, u64),
}

fn versions(path: &PathBuf, database: bool) -> Vec<Option<FileVersion>> {
  let mut paths = vec![path.clone()];
  if database {
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    paths.push(wal.into());
  }
  paths
    .into_iter()
    .map(|path| {
      std::fs::metadata(path).ok().and_then(|m| {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Some(FileVersion {
          length: m.len(),
          modified: m.modified().ok()?,
          #[cfg(unix)]
          identity: (m.dev(), m.ino()),
        })
      })
    })
    .collect()
}
