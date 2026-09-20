//! Read-only assembly of bounded, inherited Codex rollout prefixes.
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use tokn_codex_protocol::{HistoryPosition, RolloutItem};
use tokn_session_core::{AgentEvent, LoadedSessionRecords, NormalizedRecord};

use crate::CodexSessionSource;
use crate::event::CodexLine;
use crate::normalize::CodexNormalizer;
use crate::session_source::inspect_session_header;

const MAX_SEGMENTS: usize = 64;
const MAX_HEADER_BYTES: u64 = 8 * 1024 * 1024;

/// One physical range, in oldest-to-newest logical history order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexHistorySegment {
  pub path: PathBuf,
  pub end_byte_offset: Option<u64>,
  pub end_ordinal_exclusive: Option<u64>,
  header_key: serde_json::Value,
}

/// Reads the owning metadata without scanning the rollout body.
pub fn history_header(path: &Path) -> Result<CodexLine, String> {
  let file = File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
  for line in BufReader::new(file.take(MAX_HEADER_BYTES)).lines() {
    let line = line.map_err(|err| err.to_string())?;
    if line.trim().is_empty() {
      continue;
    }
    let parsed: CodexLine = serde_json::from_str(&line)
      .map_err(|err| format!("invalid Codex history metadata at {}: {err}", path.display()))?;
    if parsed.native()["type"] == "session_meta" {
      return match parsed.item() {
        RolloutItem::SessionMeta(_) => Ok(parsed),
        _ => Err(format!("invalid Codex history metadata at {}", path.display())),
      };
    }
  }
  Err(format!("missing Codex history metadata at {}", path.display()))
}

impl CodexSessionSource {
  /// Resolves a logical history using physical metadata and exact exclusive
  /// cutoffs. The native database is not needed, including after export.
  pub fn history_segments(&self, path: &Path) -> Result<Vec<CodexHistorySegment>, String> {
    let mut current = path.to_path_buf();
    let mut end: Option<HistoryPosition> = None;
    let mut segments = Vec::new();
    let mut seen = HashSet::new();
    let mut candidates = None;
    loop {
      let canonical = current.canonicalize().map_err(|err| err.to_string())?;
      if !seen.insert(canonical) || segments.len() == MAX_SEGMENTS {
        return Err("invalid Codex history lineage: cycle or excessive depth".into());
      }
      let header = history_header(&current)?;
      let RolloutItem::SessionMeta(meta) = header.item() else {
        unreachable!()
      };
      if (end.is_some() || meta.history_base.is_some()) && meta.history_mode.as_deref() != Some("paginated") {
        return Err("invalid Codex history lineage: inherited rollouts must be paginated".into());
      }
      if end.is_some() && meta.history_base.is_none() && header.ordinal() != Some(0) {
        return Err("invalid Codex history lineage: initial segment does not begin at ordinal zero".into());
      }
      segments.push(CodexHistorySegment {
        path: current.clone(),
        end_byte_offset: end.as_ref().map(|end| end.end_byte_offset),
        end_ordinal_exclusive: end.as_ref().map(|end| end.end_ordinal_exclusive),
        header_key: header_key(&header),
      });
      let Some(base) = meta.history_base.clone() else { break };
      if header.ordinal() != Some(base.end_ordinal_exclusive) {
        return Err("invalid Codex history lineage: metadata ordinal disagrees with its base".into());
      }
      if candidates.is_none() {
        let mut paths = Vec::new();
        for root in self.history_roots(path)? {
          collect_paths(&root, &mut paths)?;
        }
        candidates = Some(paths);
      }
      current = resolve_prefix(candidates.as_ref().unwrap(), &current, &base)?;
      end = Some(base);
    }
    segments.reverse();
    Ok(segments)
  }

  /// Loads complete source records atomically; missing or invalid prefixes
  /// fail instead of publishing a suffix as though it were complete history.
  pub fn load_session_records_path(
    &self,
    path: &Path,
    include_native: bool,
    max_bytes: usize,
  ) -> Result<LoadedSessionRecords, String> {
    let segments = self.history_segments(path)?;
    let owner = history_header(path)?;
    if segments
      .last()
      .is_none_or(|segment| segment.header_key != header_key(&owner))
    {
      return Err("Codex history changed while resolving its prefix".into());
    }
    let thread_spawn =
      matches!(owner.item(), RolloutItem::SessionMeta(meta) if crate::normalize::requires_thread_spawn_boundary(meta));
    let mut reference = inspect_session_header(path)?;
    if owner.native()["payload"]["id"].as_str() != Some(reference.id.as_str()) {
      return Err("Codex history owner changed while reading its metadata".into());
    }
    self.apply_indexed_metadata(std::slice::from_mut(&mut reference));
    let mut normalizer = CodexNormalizer::new_historical();
    let mut records = vec![NormalizedRecord {
      record_id: format!("session:{}", reference.id),
      native: include_native.then(|| owner.native().clone()),
      events: normalizer.normalize(owner),
    }];
    let mut consumed = 0usize;
    for segment in segments {
      let file = File::open(&segment.path).map_err(|err| err.to_string())?;
      let length = segment
        .end_byte_offset
        .unwrap_or(file.metadata().map_err(|err| err.to_string())?.len());
      let remaining = max_bytes.saturating_sub(consumed);
      if length > remaining as u64 {
        return Err("Codex history exceeds the snapshot size limit".into());
      }
      let mut bytes = Vec::new();
      file
        .take(length)
        .read_to_end(&mut bytes)
        .map_err(|err| err.to_string())?;
      if bytes.len() as u64 != length {
        return Err("Codex history changed while reading its prefix".into());
      }
      consumed += bytes.len();
      let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
      if segment.end_byte_offset.is_some() && complete != bytes.len() {
        return Err("invalid Codex history lineage: cutoff is not a complete record".into());
      }
      let mut offset = 0usize;
      let mut expected_ordinal = None;
      let mut inherited_parent = false;
      let mut saw_header = false;
      for row in bytes[..complete].split_inclusive(|byte| *byte == b'\n') {
        let row_offset = offset;
        offset += row.len();
        if row.iter().all(u8::is_ascii_whitespace) {
          continue;
        }
        let line: CodexLine = serde_json::from_slice(row).map_err(|err| {
          format!(
            "invalid Codex history at {} byte {row_offset}: {err}",
            segment.path.display()
          )
        })?;
        if segments_are_paginated(&line, expected_ordinal, segment.end_ordinal_exclusive)? {
          expected_ordinal = line.ordinal().and_then(|ordinal| ordinal.checked_add(1));
        }
        if let RolloutItem::SessionMeta(meta) = line.item() {
          if !saw_header {
            if header_key(&line) != segment.header_key {
              return Err("Codex history changed while reading its prefix".into());
            }
            saw_header = true;
            inherited_parent = thread_spawn && meta.id.as_deref() != Some(reference.id.as_str());
          }
          continue;
        }
        let native = include_native.then(|| line.native().clone());
        // A trigger in the referenced parent's own history does not begin the
        // child's work. Same-thread continuation prefixes still pass through
        // the child's historical boundary normalizer.
        let events = if inherited_parent {
          Vec::new()
        } else {
          normalizer.normalize(line)
        };
        reference.message_count += events
          .iter()
          .filter(|event| matches!(event, AgentEvent::Message(_)))
          .count();
        records.push(NormalizedRecord {
          record_id: format!("segment:{}:{row_offset}", segment.path.display()),
          native,
          events,
        });
      }
      if !saw_header {
        return Err("Codex history metadata is not yet complete".into());
      }
      if segment.end_ordinal_exclusive.is_some() && expected_ordinal != segment.end_ordinal_exclusive {
        return Err("invalid Codex history lineage: cutoff ordinal disagrees with its bytes".into());
      }
    }
    Ok(LoadedSessionRecords {
      reference,
      records,
      history_status: normalizer.history_status(),
    })
  }
}

fn header_key(line: &CodexLine) -> serde_json::Value {
  let payload = &line.native()["payload"];
  serde_json::json!({
    "ordinal": line.ordinal(),
    "id": payload["id"],
    "history_mode": payload["history_mode"],
    "history_base": payload["history_base"],
  })
}

fn segments_are_paginated(line: &CodexLine, expected: Option<u64>, end: Option<u64>) -> Result<bool, String> {
  let paginated = expected.is_some()
    || end.is_some()
    || matches!(line.item(), RolloutItem::SessionMeta(meta) if meta.history_mode.as_deref() == Some("paginated"));
  if paginated && (line.ordinal().is_none() || expected.is_some_and(|expected| line.ordinal() != Some(expected))) {
    return Err("invalid Codex history lineage: missing or out-of-order ordinal".into());
  }
  Ok(paginated)
}

fn collect_paths(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
  if !root.exists() {
    return Ok(());
  }
  for entry in std::fs::read_dir(root).map_err(|err| err.to_string())? {
    let entry = entry.map_err(|err| err.to_string())?;
    let kind = entry.file_type().map_err(|err| err.to_string())?;
    if kind.is_dir() {
      collect_paths(&entry.path(), paths)?;
    } else if kind.is_file() && entry.path().extension().is_some_and(|extension| extension == "jsonl") {
      paths.push(entry.path());
    }
  }
  Ok(())
}

fn resolve_prefix(paths: &[PathBuf], current: &Path, base: &HistoryPosition) -> Result<PathBuf, String> {
  if base.end_ordinal_exclusive == 0 || base.end_byte_offset == 0 {
    return Err("invalid Codex history lineage: empty prefix cutoff".into());
  }
  let current = current.canonicalize().map_err(|err| err.to_string())?;
  let mut matches = Vec::new();
  for path in paths {
    if path.canonicalize().is_ok_and(|path| path == current) {
      continue;
    }
    // Native filenames carry their owning thread ID. Exported arbitrary names
    // are still supported, while unrelated native headers are never reopened.
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
    if name.starts_with("rollout-") && !name.contains(&base.thread_id) {
      continue;
    }
    let Ok(header) = history_header(path) else { continue };
    let RolloutItem::SessionMeta(meta) = header.item() else {
      continue;
    };
    if meta.id.as_deref() != Some(base.thread_id.as_str())
      || meta.history_mode.as_deref() != Some("paginated")
      || !header
        .ordinal()
        .is_some_and(|ordinal| ordinal < base.end_ordinal_exclusive)
    {
      continue;
    }
    if cutoff_matches(path, base)? {
      matches.push(path.clone());
    }
  }
  match matches.as_slice() {
    [path] => Ok(path.clone()),
    [] => Err(format!(
      "Codex history prefix is unavailable for {} at ordinal {}",
      base.thread_id, base.end_ordinal_exclusive
    )),
    _ => Err(format!(
      "Codex history prefix is ambiguous for {} at ordinal {}",
      base.thread_id, base.end_ordinal_exclusive
    )),
  }
}

fn cutoff_matches(path: &Path, base: &HistoryPosition) -> Result<bool, String> {
  let mut file = File::open(path).map_err(|err| err.to_string())?;
  if file.metadata().map_err(|err| err.to_string())?.len() < base.end_byte_offset {
    return Ok(false);
  }
  // Only inspect the final bounded record; full ordinal continuity is checked
  // by the atomic loader. This keeps dependency discovery independent of size.
  let mut window = 64 * 1024;
  loop {
    let start = base.end_byte_offset.saturating_sub(window);
    file.seek(SeekFrom::Start(start)).map_err(|err| err.to_string())?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
      .take(base.end_byte_offset - start)
      .read_to_end(&mut bytes)
      .map_err(|err| err.to_string())?;
    if bytes.last() != Some(&b'\n') {
      return Ok(false);
    }
    let end = bytes.len() - 1;
    let record_start = bytes[..end]
      .iter()
      .rposition(|byte| *byte == b'\n')
      .map_or(0, |index| index + 1);
    if record_start == 0 && start != 0 {
      if window == MAX_HEADER_BYTES {
        return Ok(false);
      }
      window = (window * 2).min(MAX_HEADER_BYTES);
      continue;
    }
    let Ok(line) = serde_json::from_slice::<CodexLine>(&bytes[record_start..end]) else {
      return Ok(false);
    };
    return Ok(line.ordinal().and_then(|ordinal| ordinal.checked_add(1)) == Some(base.end_ordinal_exclusive));
  }
}
