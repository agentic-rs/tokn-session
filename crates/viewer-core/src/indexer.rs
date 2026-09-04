//! One process owns provider indexing for each durable database. The OS releases
//! the lease after a crash; readers never need to delete a stale lock file.
use std::{
  fs::{File, OpenOptions, TryLockError},
  io::Write,
  path::{Path, PathBuf},
};

#[derive(Clone)]
pub(crate) struct IndexerLock {
  path: PathBuf,
  retry: PathBuf,
}

pub(crate) struct IndexerLease {
  _file: Option<File>,
}

impl IndexerLock {
  pub(crate) fn new(database: &Path) -> Result<Self, String> {
    let database = database.canonicalize().map_err(|e| e.to_string())?;
    let mut path = database.as_os_str().to_os_string();
    path.push(".indexer.lock");
    let mut retry = database.as_os_str().to_os_string();
    retry.push(".indexer.retry");
    Ok(Self {
      path: path.into(),
      retry: retry.into(),
    })
  }

  pub(crate) fn try_acquire(&self) -> Result<Option<IndexerLease>, String> {
    let file = OpenOptions::new()
      .read(true)
      .write(true)
      .create(true)
      .truncate(false)
      .open(&self.path)
      .map_err(|e| e.to_string())?;
    match file.try_lock() {
      Ok(()) => Ok(Some(IndexerLease { _file: Some(file) })),
      Err(TryLockError::WouldBlock) => Ok(None),
      Err(TryLockError::Error(error)) => Err(error.to_string()),
    }
  }

  // An append is an atomic, persistent request generation. Only explicit user
  // retries write here; ordinary feed events remain advisory and process-local.
  pub(crate) fn request_retry(&self) -> Result<(), String> {
    OpenOptions::new()
      .create(true)
      .append(true)
      .open(&self.retry)
      .and_then(|mut file| file.write_all(b"\n"))
      .map_err(|e| e.to_string())
  }

  pub(crate) fn retry_generation(&self) -> Result<u64, String> {
    match self.retry.metadata() {
      Ok(metadata) => Ok(metadata.len()),
      Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
      Err(error) => Err(error.to_string()),
    }
  }
}

impl IndexerLease {
  pub(crate) fn in_memory() -> Self {
    Self { _file: None }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::Arc;

  #[test]
  fn lease_excludes_other_handles_and_survives_worker_clone() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("index.sqlite");
    File::create(&database).unwrap();
    let lock = IndexerLock::new(&database).unwrap();
    let owner = Arc::new(lock.try_acquire().unwrap().unwrap());
    let worker = owner.clone();
    assert!(lock.try_acquire().unwrap().is_none());
    drop(owner);
    assert!(lock.try_acquire().unwrap().is_none());
    drop(worker);
    assert!(lock.try_acquire().unwrap().is_some());
    assert_eq!(lock.retry_generation().unwrap(), 0);
    lock.request_retry().unwrap();
    lock.request_retry().unwrap();
    assert_eq!(lock.retry_generation().unwrap(), 2);
  }
}
