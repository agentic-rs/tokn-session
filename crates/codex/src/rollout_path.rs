use std::path::Path;

pub(crate) fn rollout_thread_id(path: &Path) -> Option<&str> {
  rollout_parts(path).map(|(owner, _)| owner)
}

/// Continuation filenames carry a physical segment UUID after the owner UUID.
/// Require the filename owner to agree with metadata before trusting that alias.
pub(crate) fn rollout_segment_matches(path: &Path, owner: Option<&str>, segment: &str) -> bool {
  let Some((filename_owner, suffix)) = rollout_parts(path) else {
    return false;
  };
  owner == Some(filename_owner) && suffix.strip_prefix('_').is_some_and(|id| is_uuid(id) && id == segment)
}

fn rollout_parts(path: &Path) -> Option<(&str, &str)> {
  let stem = path.file_stem()?.to_str()?.strip_prefix("rollout-")?;
  // Native names contain a 19-byte timestamp, '-', then the owning UUID.
  // New segments add a second UUID separated by '_'.
  let rest = stem.get(20..)?;
  let owner = rest.get(..36)?;
  let suffix = rest.get(36..)?;
  (is_uuid(owner) && (suffix.is_empty() || suffix.starts_with('_'))).then_some((owner, suffix))
}

fn is_uuid(value: &str) -> bool {
  value.len() == 36
    && value.bytes().enumerate().all(|(index, byte)| {
      if matches!(index, 8 | 13 | 18 | 23) {
        byte == b'-'
      } else {
        byte.is_ascii_hexdigit()
      }
    })
}
