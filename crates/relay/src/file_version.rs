use std::{path::PathBuf, time::SystemTime};
#[derive(Clone, PartialEq, Eq)]
pub struct FileVersion {
  pub length: u64,
  modified: SystemTime,
  #[cfg(unix)]
  pub identity: (u64, u64),
}

pub fn versions(path: &PathBuf, database: bool) -> Vec<Option<FileVersion>> {
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
