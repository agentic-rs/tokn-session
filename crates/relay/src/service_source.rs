use std::{
  path::PathBuf,
  sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
  },
  time::SystemTime,
};

use tokn_session_core::{NormalizedRecord, Provider};
use tokn_session_opencode::{OpenCodeSessionCache, OpenCodeSessionSource};

use crate::{RecordOperation, RelayRecord, SessionContext, service_protocol::CatalogEntry, tailer::FileState};

#[cfg(test)]
mod tests;

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
  database_cache: OpenCodeSessionCache,
  database_records: Vec<Arc<NormalizedRecord>>,
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
      database_cache: OpenCodeSessionCache::with_max_source_bytes(crate::service_protocol::MAX_SNAPSHOT_BYTES),
      database_records: Vec::new(),
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
    if self.database.is_some() {
      return self.poll_database(version);
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

  fn poll_database(&mut self, version: Vec<Option<FileVersion>>) -> Result<bool, String> {
    let loaded = self.database.as_ref().unwrap().load_session_records_cached_exact(
      &self.snapshot.entry.header.id,
      self.native,
      &mut self.database_cache,
    )?;
    let mut prefix = 0;
    for (old, new) in self.database_records.iter().zip(&loaded.records) {
      // Unchanged rows retain their allocation. Changed raw JSON can still
      // produce identical output (e.g. unknown fields with native disabled).
      if !Arc::ptr_eq(old, new)
        && serde_json::to_value(old.as_ref()).map_err(|e| e.to_string())?
          != serde_json::to_value(new.as_ref()).map_err(|e| e.to_string())?
      {
        break;
      }
      prefix += 1;
    }
    #[cfg(unix)]
    let replaced = self.version.first().and_then(Option::as_ref).map(|v| v.identity)
      != version.first().and_then(Option::as_ref).map(|v| v.identity);
    #[cfg(not(unix))]
    let replaced = false;
    let reset = replaced || prefix < self.database_records.len();
    let header_changed = loaded.header != self.snapshot.entry.header;
    let changed =
      reset || loaded.records.len() != self.database_records.len() || header_changed || self.snapshot.error.is_some();
    if !changed {
      self.database_records = loaded.records;
      self.version = version;
      return Ok(false);
    }
    let context = SessionContext::from_session_ref(&loaded.reference);
    let mut records = Vec::new();
    let mut bytes = if reset { 0 } else { self.bytes };
    for record in loaded.records.iter().skip(if reset { 0 } else { prefix }) {
      let record = RelayRecord {
        path: loaded.reference.path.clone(),
        topic: format!("opencode.{}", context.session_id),
        session: context.clone(),
        operation: RecordOperation::Upsert,
        record: record.as_ref().clone(),
      };
      bytes = bytes.saturating_add(serde_json::to_vec(&record).map_err(|e| e.to_string())?.len());
      if bytes > crate::service_protocol::MAX_SNAPSHOT_BYTES {
        return Err("Relay session exceeds the snapshot memory limit".into());
      }
      records.push(Arc::new(record));
    }
    if reset {
      self.snapshot.generation = generation();
      self.snapshot.records.clear();
    }
    self.snapshot.records.extend(records);
    self.snapshot.entry.header = loaded.header;
    self.snapshot.revision += 1;
    self.snapshot.error = None;
    self.database_records = loaded.records;
    self.bytes = bytes;
    self.version = version;
    Ok(true)
  }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct FileVersion {
  pub(crate) length: u64,
  modified: SystemTime,
  #[cfg(unix)]
  pub(crate) identity: (u64, u64),
}

pub(crate) fn versions(path: &PathBuf, database: bool) -> Vec<Option<FileVersion>> {
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
