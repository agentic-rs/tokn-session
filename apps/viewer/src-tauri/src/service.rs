use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::Serialize;
use serde_json::{Value, json};
use tokn_session_client::SessionHeader;
use tokn_session_core::{
  AgentEvent, LifecycleOutcome, LoadedSession, Phase, Provider, Role, TerminalAction, ToolCallEvent, ToolKind,
  ToolOperation, ToolOperationStatus, ToolSummary, UsageKind, assemble_tool_operations,
};
use tokn_session_render::render_event_summary;

use crate::model::{
  EventDetail, EventPage, EventPageRequest, EventSummary, ListSessionChildrenRequest, ListSessionChildrenResponse,
  ListSessionsRequest, ListSessionsResponse, LoadEventDetailRequest, PageDirection, ReasoningCardSummary,
  SessionLocator, SessionSummary, SourceError, ToolCardSummary, ToolOutputPreview, ToolOutputSection, UsageCardSummary,
  ViewerProvider, bounded_limit, decode_event_cursor, decode_event_key, decode_list_cursor, decode_session_key,
  encode_event_cursor, encode_event_key, encode_list_cursor, encode_session_key, parse_updated_at_ms, requested_offset,
};
use crate::repository::{NativeRepository, ViewerRepository};

const MAX_DETAIL_VALUE_BYTES: usize = 512 * 1024;
const MAX_MESSAGE_SUMMARY_CHARS: usize = 16 * 1024;
const MAX_SESSION_PREVIEW_CHARS: usize = 240;
const MAX_SESSION_TITLE_CHARS: usize = 160;
const MAX_AGENT_IDENTITY_CHARS: usize = 160;
const MAX_REASONING_CARD_PREVIEW_CHARS: usize = 240;
const MAX_TECHNICAL_SUMMARY_CHARS: usize = 500;
const MAX_TOOL_CARD_STRING_CHARS: usize = 500;
const MAX_TOOL_NAME_CHARS: usize = 120;
const MAX_TOOL_OUTPUT_BYTES: usize = 64 * 1024;
const TOOL_OUTPUT_TRUNCATION_MARKER: &str = "\n\u{2026} output truncated \u{2026}\n";

#[derive(Clone)]
pub(crate) struct ViewerService {
  repository: Arc<dyn ViewerRepository>,
  session_header_cache: Arc<Mutex<HashMap<SessionLocator, CachedSessionHeader>>>,
  session_header_gates: Arc<Mutex<HashMap<SessionLocator, Arc<Mutex<()>>>>>,
  loaded_session_cache: Arc<Mutex<Option<CachedSession>>>,
}

struct CachedSessionHeader {
  source_revision: SourceRevision,
  header: SessionHeader,
}

struct CachedSession {
  locator: SessionLocator,
  source_revision: SourceRevision,
  loaded: Arc<LoadedSession>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceRevision {
  files: Vec<Option<FileRevision>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileRevision {
  len: u64,
  modified: Option<SystemTime>,
  created: Option<SystemTime>,
}

#[derive(Clone)]
struct SessionListCandidate {
  provider: ViewerProvider,
  header: SessionHeader,
  child_count: usize,
  is_subagent: bool,
}

struct SessionRelationIndex {
  headers: Vec<SessionHeader>,
  parent_indices: Vec<Option<usize>>,
  child_counts: Vec<usize>,
}

/// One visible historical timeline row. Tool operations intentionally retain
/// their source event index as the stable detail key while hiding intermediate
/// invocation/progress/result fragments from the presentation timeline.
enum TimelineEntry {
  Event {
    source_event_index: usize,
  },
  ToolOperation {
    source_event_index: usize,
    operation: ToolOperation,
  },
}

impl ViewerService {
  pub fn native() -> Self {
    Self {
      repository: Arc::new(NativeRepository),
      session_header_cache: Arc::new(Mutex::new(HashMap::new())),
      session_header_gates: Arc::new(Mutex::new(HashMap::new())),
      loaded_session_cache: Arc::new(Mutex::new(None)),
    }
  }

  #[cfg(test)]
  fn new(repository: Arc<dyn ViewerRepository>) -> Self {
    Self {
      repository,
      session_header_cache: Arc::new(Mutex::new(HashMap::new())),
      session_header_gates: Arc::new(Mutex::new(HashMap::new())),
      loaded_session_cache: Arc::new(Mutex::new(None)),
    }
  }

  pub fn list_sessions(&self, request: ListSessionsRequest) -> Result<ListSessionsResponse, String> {
    let limit = bounded_limit(request.limit)?;
    let offset = requested_offset(request.cursor.as_deref(), request.offset, decode_list_cursor)?.unwrap_or(0);
    let providers = selected_providers(&request.query.providers);
    let search = request
      .query
      .search
      .as_deref()
      .map(str::trim)
      .filter(|value| !value.is_empty())
      .map(str::to_lowercase);
    let mut candidates = Vec::new();
    let mut source_errors = Vec::new();

    for provider in providers {
      let headers = match self.repository.list_session_headers(provider) {
        Ok(headers) => headers,
        Err(message) => {
          record_source_error(&mut source_errors, provider, message);
          continue;
        }
      };

      let relations = session_relation_index(provider, headers, &mut source_errors);
      for (index, header) in relations.headers.into_iter().enumerate() {
        // A child is hidden from the root roster only when its parent is a
        // present, canonical header and its relation does not make a cycle.
        // That deliberately keeps orphaned and cyclic records discoverable.
        if relations.parent_indices[index].is_some() {
          continue;
        }
        candidates.push(SessionListCandidate {
          provider,
          header,
          child_count: relations.child_counts[index],
          is_subagent: false,
        });
      }
    }

    if let Some(search) = search.as_deref() {
      let mut matches = Vec::new();
      for candidate in candidates {
        let summary = match session_summary_with_child_count(
          candidate.provider,
          candidate.header.clone(),
          candidate.child_count,
          candidate.is_subagent,
        ) {
          Ok(summary) => summary,
          Err(message) => {
            record_source_error(&mut source_errors, candidate.provider, message);
            continue;
          }
        };
        if matches_search(&summary, Some(search)) {
          matches.push(candidate);
          continue;
        }
        if summary.preview.is_some() {
          continue;
        }
        let hydrated = self.hydrate_session_header(candidate.provider, candidate.header.clone());
        let summary = match session_summary_with_child_count(
          candidate.provider,
          hydrated.clone(),
          candidate.child_count,
          candidate.is_subagent,
        ) {
          Ok(summary) => summary,
          Err(message) => {
            record_source_error(&mut source_errors, candidate.provider, message);
            continue;
          }
        };
        if matches_search(&summary, Some(search)) {
          matches.push(SessionListCandidate {
            header: hydrated,
            ..candidate
          });
        }
      }
      sort_session_candidates(&mut matches);
      let start = offset.min(matches.len());
      let end = start.saturating_add(limit).min(matches.len());
      let next_cursor = (end < matches.len()).then(|| encode_list_cursor(end));
      let mut sessions = Vec::with_capacity(end - start);
      for candidate in matches[start..end].iter().cloned() {
        let header = if present_string(candidate.header.title.as_deref()).is_none()
          && present_string(candidate.header.preview.as_deref()).is_none()
        {
          self.hydrate_session_header(candidate.provider, candidate.header)
        } else {
          candidate.header
        };
        match session_summary_with_child_count(candidate.provider, header, candidate.child_count, candidate.is_subagent)
        {
          Ok(summary) => sessions.push(summary),
          Err(message) => record_source_error(&mut source_errors, candidate.provider, message),
        }
      }
      return Ok(ListSessionsResponse {
        sessions,
        next_cursor,
        source_errors,
      });
    }

    sort_session_candidates(&mut candidates);
    let start = offset.min(candidates.len());
    let end = start.saturating_add(limit).min(candidates.len());
    let next_cursor = (end < candidates.len()).then(|| encode_list_cursor(end));
    let mut sessions = Vec::with_capacity(end - start);
    for candidate in candidates[start..end].iter().cloned() {
      let header = if present_string(candidate.header.title.as_deref()).is_none()
        && present_string(candidate.header.preview.as_deref()).is_none()
      {
        self.hydrate_session_header(candidate.provider, candidate.header)
      } else {
        candidate.header
      };
      match session_summary_with_child_count(candidate.provider, header, candidate.child_count, candidate.is_subagent) {
        Ok(summary) => sessions.push(summary),
        Err(message) => record_source_error(&mut source_errors, candidate.provider, message),
      }
    }

    Ok(ListSessionsResponse {
      sessions,
      next_cursor,
      source_errors,
    })
  }

  /// Lists a bounded page of direct child headers without reading any child
  /// conversation body. The opaque parent key binds the request to one
  /// provider and one source record, so raw provider-local IDs never connect
  /// sessions from different providers.
  pub fn list_session_children(
    &self,
    request: ListSessionChildrenRequest,
  ) -> Result<ListSessionChildrenResponse, String> {
    let limit = bounded_limit(request.limit)?;
    let offset = requested_offset(request.cursor.as_deref(), request.offset, decode_list_cursor)?.unwrap_or(0);
    let parent_locator = decode_session_key(&request.parent_session_key)?;
    let headers = self.repository.list_session_headers(parent_locator.provider)?;
    let mut ignored_errors = Vec::new();
    let relations = session_relation_index(parent_locator.provider, headers, &mut ignored_errors);
    let parent_index = relations
      .headers
      .iter()
      .position(|header| locator_for_header(parent_locator.provider, header) == parent_locator)
      .ok_or_else(|| "session key no longer matches its source record".to_string())?;

    let mut candidates = relations
      .headers
      .into_iter()
      .enumerate()
      .filter_map(|(index, header)| {
        (relations.parent_indices[index] == Some(parent_index)).then_some(SessionListCandidate {
          provider: parent_locator.provider,
          header,
          child_count: relations.child_counts[index],
          is_subagent: true,
        })
      })
      .collect::<Vec<_>>();
    sort_session_candidates(&mut candidates);

    let start = offset.min(candidates.len());
    let end = start.saturating_add(limit).min(candidates.len());
    let next_cursor = (end < candidates.len()).then(|| encode_list_cursor(end));
    let sessions = candidates[start..end]
      .iter()
      .cloned()
      .map(|candidate| {
        session_summary_with_child_count(
          candidate.provider,
          candidate.header,
          candidate.child_count,
          candidate.is_subagent,
        )
      })
      .collect::<Result<Vec<_>, _>>()?;

    Ok(ListSessionChildrenResponse { sessions, next_cursor })
  }

  fn hydrate_session_header(&self, provider: ViewerProvider, mut header: SessionHeader) -> SessionHeader {
    let locator = locator_for_header(provider, &header);
    let Some(initial_revision) = source_revision(&locator) else {
      return self
        .repository
        .hydrate_session_header(provider, header.clone())
        .unwrap_or(header);
    };
    if apply_cached_session_header(&self.session_header_cache, &locator, &initial_revision, &mut header) {
      return header;
    }

    let gate = match self.session_header_gates.lock() {
      Ok(mut gates) => Arc::clone(gates.entry(locator.clone()).or_insert_with(|| Arc::new(Mutex::new(())))),
      Err(_) => {
        return self
          .repository
          .hydrate_session_header(provider, header.clone())
          .unwrap_or(header);
      }
    };
    let Ok(_gate) = gate.lock() else {
      return self
        .repository
        .hydrate_session_header(provider, header.clone())
        .unwrap_or(header);
    };
    let Some(revision_before) = source_revision(&locator) else {
      return self
        .repository
        .hydrate_session_header(provider, header.clone())
        .unwrap_or(header);
    };
    if apply_cached_session_header(&self.session_header_cache, &locator, &revision_before, &mut header) {
      return header;
    }

    // Search requests overlap while the user is typing. A per-session gate
    // prevents duplicate scans without letting an old slow transcript block
    // unrelated providers or the currently visible result.
    let hydrated = match self.repository.hydrate_session_header(provider, header.clone()) {
      Ok(hydrated) => hydrated,
      Err(_) => return header,
    };
    if let Some(revision_after) = source_revision(&locator)
      && revision_before == revision_after
      && let Ok(mut cache) = self.session_header_cache.lock()
    {
      cache.insert(
        locator,
        CachedSessionHeader {
          source_revision: revision_after,
          header: hydrated.clone(),
        },
      );
    }
    hydrated
  }

  pub fn load_event_page(&self, request: EventPageRequest) -> Result<EventPage, String> {
    let limit = bounded_limit(request.limit)?;
    let locator = decode_session_key(&request.session_key)?;
    let loaded = self.load_verified(&locator)?;
    let timeline = timeline_entries(&loaded.events);
    let total_events = timeline.len();
    let requested = requested_offset(request.cursor.as_deref(), request.offset, decode_event_cursor)?;
    let boundary = requested.unwrap_or(match request.direction {
      PageDirection::Forward => 0,
      PageDirection::Backward => total_events,
    });
    let (start, end) = event_page_bounds(total_events, boundary, request.direction, limit)?;
    let events = timeline[start..end]
      .iter()
      .map(|entry| match entry {
        TimelineEntry::Event { source_event_index } => {
          event_summary(&loaded.events, *source_event_index, &loaded.events[*source_event_index])
        }
        TimelineEntry::ToolOperation {
          source_event_index,
          operation,
        } => tool_operation_event_summary(*source_event_index, operation),
      })
      .collect();

    Ok(EventPage {
      events,
      next_cursor: (end < total_events).then(|| encode_event_cursor(end)),
      previous_cursor: (start > 0).then(|| encode_event_cursor(start)),
      total_events,
      history_status: loaded.history_status.into(),
    })
  }

  pub fn load_event_detail(&self, request: LoadEventDetailRequest) -> Result<EventDetail, String> {
    let locator = decode_session_key(&request.session_key)?;
    let source_event_index = decode_event_key(&request.event_key)?;
    let loaded = self.load_verified(&locator)?;
    let entry = timeline_entry_for_source(&loaded.events, source_event_index)
      .ok_or_else(|| "event key is outside the session".to_string())?;

    match entry {
      TimelineEntry::Event { source_event_index } => {
        let event = &loaded.events[source_event_index];
        event_detail(
          encode_event_key(source_event_index),
          event,
          &loaded.events,
          source_event_index,
        )
      }
      TimelineEntry::ToolOperation {
        source_event_index,
        operation,
      } => tool_operation_detail(encode_event_key(source_event_index), &operation, &loaded.events),
    }
  }

  fn load_verified(&self, locator: &SessionLocator) -> Result<Arc<LoadedSession>, String> {
    let revision_before = source_revision(locator);
    if let Some(revision) = revision_before.as_ref() {
      let cache = self
        .loaded_session_cache
        .lock()
        .map_err(|_| "loaded session cache lock is poisoned".to_string())?;
      if let Some(cached) = cache
        .as_ref()
        .filter(|cached| cached.locator == *locator && cached.source_revision == *revision)
      {
        return Ok(Arc::clone(&cached.loaded));
      }
    }

    let loaded = Arc::new(self.repository.load_session(locator)?);
    if loaded.reference.id != locator.session_id {
      return Err("session key no longer matches its source record".to_string());
    }

    let revision_after = source_revision(locator);
    let mut cache = self
      .loaded_session_cache
      .lock()
      .map_err(|_| "loaded session cache lock is poisoned".to_string())?;
    *cache = match (revision_before, revision_after) {
      (Some(before), Some(after)) if before == after => Some(CachedSession {
        locator: locator.clone(),
        source_revision: after,
        loaded: Arc::clone(&loaded),
      }),
      _ => None,
    };
    Ok(loaded)
  }
}

fn event_detail(
  event_key: String,
  event: &AgentEvent,
  events: &[AgentEvent],
  source_event_index: usize,
) -> Result<EventDetail, String> {
  let is_hidden = event.is_hidden();
  if is_hidden {
    return Ok(EventDetail {
      event_key,
      event: json!({
        "type": normalized_event_type(event),
        "provider": provider_for_event(event).as_str(),
        "redacted": true,
      }),
      native: None,
      is_hidden: true,
      tool_output: None,
    });
  }
  if matches!(event, AgentEvent::Reasoning(reasoning) if reasoning.redacted == Some(true)) {
    return Ok(EventDetail {
      event_key,
      event: json!({
        "type": "reasoning",
        "provider": provider_for_event(event).as_str(),
        "redacted": true,
      }),
      native: None,
      is_hidden: false,
      tool_output: None,
    });
  }

  let native = native_detail(event)
    .map(|value| bounded_detail_value(value, "provider_native"))
    .transpose()?;
  let mut normalized =
    serde_json::to_value(event).map_err(|error| format!("failed to serialize normalized event: {error}"))?;
  remove_embedded_native(&mut normalized);
  let normalized = bounded_detail_value(normalized, "normalized_event")?;
  let tool_output = tool_output_preview(events, source_event_index);
  Ok(EventDetail {
    event_key,
    event: normalized,
    native,
    is_hidden: false,
    tool_output,
  })
}

fn tool_operation_detail(
  event_key: String,
  operation: &ToolOperation,
  events: &[AgentEvent],
) -> Result<EventDetail, String> {
  let mut normalized = serde_json::to_value(operation)
    .map_err(|error| format!("failed to serialize normalized tool operation: {error}"))?;
  remove_embedded_native(&mut normalized);
  let normalized = bounded_detail_value(normalized, "normalized_tool_operation")?;
  let native = tool_operation_native_detail(operation, events)
    .map(|value| bounded_detail_value(value, "provider_native"))
    .transpose()?;
  let tool_output = tool_operation_output_preview(operation);

  Ok(EventDetail {
    event_key,
    event: normalized,
    native,
    is_hidden: false,
    tool_output,
  })
}

fn tool_operation_native_detail(operation: &ToolOperation, events: &[AgentEvent]) -> Option<Value> {
  let records = operation
    .source_event_indices
    .iter()
    .filter_map(|&source_event_index| {
      let AgentEvent::ToolCall(event) = events.get(source_event_index)? else {
        return None;
      };
      event.native.as_ref().map(|native| {
        json!({
          "event_key": encode_event_key(source_event_index),
          "record_kind": serialized_label(event.record_kind),
          "timestamp": event.timestamp,
          "native": native,
        })
      })
    })
    .collect::<Vec<_>>();
  (!records.is_empty()).then(|| json!({ "source_records": records }))
}

fn tool_operation_output_preview(operation: &ToolOperation) -> Option<ToolOutputPreview> {
  let output = operation.output.as_ref().filter(|value| !value.is_null())?;
  let sections = project_output(output, 0);
  let source_event_index = *operation.source_event_indices.first()?;
  (!sections.is_empty()).then(|| bound_tool_output(sections, source_event_index))
}

fn apply_cached_session_header(
  cache: &Mutex<HashMap<SessionLocator, CachedSessionHeader>>,
  locator: &SessionLocator,
  revision: &SourceRevision,
  header: &mut SessionHeader,
) -> bool {
  let Ok(cache) = cache.lock() else {
    return false;
  };
  let Some(cached) = cache.get(locator).filter(|cached| cached.source_revision == *revision) else {
    return false;
  };
  if present_string(header.title.as_deref()).is_none() {
    header.title.clone_from(&cached.header.title);
  }
  if present_string(header.preview.as_deref()).is_none() {
    header.preview.clone_from(&cached.header.preview);
  }
  true
}

fn source_revision(locator: &SessionLocator) -> Option<SourceRevision> {
  let mut paths = vec![locator.source_path.clone()];
  if locator.provider == ViewerProvider::OpenCode {
    for suffix in ["-wal", "-shm"] {
      let mut sidecar = locator.source_path.as_os_str().to_os_string();
      sidecar.push(suffix);
      paths.push(sidecar.into());
    }
  }

  let primary = file_revision(&paths[0])?;
  let mut files = vec![Some(primary)];
  files.extend(paths[1..].iter().map(|path| file_revision(path)));
  Some(SourceRevision { files })
}

fn file_revision(path: &Path) -> Option<FileRevision> {
  let metadata = std::fs::metadata(path).ok()?;
  Some(FileRevision {
    len: metadata.len(),
    modified: metadata.modified().ok(),
    created: metadata.created().ok(),
  })
}

fn selected_providers(requested: &[ViewerProvider]) -> Vec<ViewerProvider> {
  if requested.is_empty() {
    return ViewerProvider::ALL.to_vec();
  }
  ViewerProvider::ALL
    .into_iter()
    .filter(|provider| requested.contains(provider))
    .collect()
}

fn record_source_error(errors: &mut Vec<SourceError>, provider: ViewerProvider, message: String) {
  if errors.iter().all(|error| error.provider != provider) {
    errors.push(SourceError { provider, message });
  }
}

fn validate_session_header(provider: ViewerProvider, header: &SessionHeader) -> Result<(), String> {
  encode_session_key(&locator_for_header(provider, header)).map(|_| ())
}

fn session_relation_index(
  provider: ViewerProvider,
  headers: Vec<SessionHeader>,
  source_errors: &mut Vec<SourceError>,
) -> SessionRelationIndex {
  let mut valid_headers = Vec::with_capacity(headers.len());
  for header in headers {
    if let Err(message) = validate_session_header(provider, &header) {
      record_source_error(source_errors, provider, message);
      continue;
    }
    valid_headers.push(header);
  }
  let headers = canonical_session_headers(valid_headers);
  let indices_by_id = headers
    .iter()
    .enumerate()
    .map(|(index, header)| (header.id.as_str(), index))
    .collect::<HashMap<_, _>>();
  let mut parent_indices = vec![None; headers.len()];

  for (child_index, header) in headers.iter().enumerate() {
    let Some(parent_id) = header.parent_session_id.as_deref() else {
      continue;
    };
    let Some(&parent_index) = indices_by_id.get(parent_id) else {
      continue;
    };
    if parent_index == child_index || relation_would_cycle(&parent_indices, child_index, parent_index) {
      continue;
    }
    parent_indices[child_index] = Some(parent_index);
  }

  let mut child_counts = vec![0; headers.len()];
  for parent_index in parent_indices.iter().flatten() {
    child_counts[*parent_index] += 1;
  }

  SessionRelationIndex {
    headers,
    parent_indices,
    child_counts,
  }
}

fn canonical_session_headers(mut headers: Vec<SessionHeader>) -> Vec<SessionHeader> {
  // Match the client tree loader's policy: when a provider has more than one
  // header with the same ID, the newest provider timestamp owns that
  // provider-local identity. Filesystem mtime is deliberately not involved:
  // it can change long after the provider wrote the rollout. The source path
  // remains part of the opaque key, but cannot resolve an ambiguous parent ID
  // on its own.
  headers.sort_by(|left, right| {
    right
      .timestamp
      .cmp(&left.timestamp)
      .then_with(|| right.path.cmp(&left.path))
  });
  let mut canonical_ids = HashSet::new();
  headers.retain(|header| canonical_ids.insert(header.id.clone()));
  headers.sort_by(compare_session_headers);
  headers
}

fn relation_would_cycle(parent_indices: &[Option<usize>], child_index: usize, parent_index: usize) -> bool {
  let mut current = Some(parent_index);
  while let Some(index) = current {
    if index == child_index {
      return true;
    }
    current = parent_indices[index];
  }
  false
}

fn sort_session_candidates(candidates: &mut [SessionListCandidate]) {
  candidates.sort_by(|left, right| {
    compare_session_headers(&left.header, &right.header)
      .then_with(|| left.provider.as_str().cmp(right.provider.as_str()))
  });
}

fn compare_session_headers(left: &SessionHeader, right: &SessionHeader) -> std::cmp::Ordering {
  right
    .updated_at_ms
    .cmp(&left.updated_at_ms)
    .then_with(|| right.timestamp.cmp(&left.timestamp))
    .then_with(|| left.id.cmp(&right.id))
    .then_with(|| left.path.cmp(&right.path))
}

fn locator_for_header(provider: ViewerProvider, header: &SessionHeader) -> SessionLocator {
  SessionLocator {
    version: 1,
    provider,
    session_id: header.id.clone(),
    source_path: header.path.clone(),
  }
}

#[cfg(test)]
fn session_summary(provider: ViewerProvider, header: SessionHeader) -> Result<SessionSummary, String> {
  session_summary_with_child_count(provider, header, 0, false)
}

fn session_summary_with_child_count(
  provider: ViewerProvider,
  header: SessionHeader,
  child_count: usize,
  is_subagent: bool,
) -> Result<SessionSummary, String> {
  let locator = locator_for_header(provider, &header);
  let project = header.cwd.as_deref().and_then(path_name).map(str::to_string);
  let title = normalize_session_text(header.title, MAX_SESSION_TITLE_CHARS);
  let preview = normalize_session_text(header.preview, MAX_SESSION_PREVIEW_CHARS);
  let agent_path = normalize_session_text(header.agent_path, MAX_AGENT_IDENTITY_CHARS);
  let agent_nickname = normalize_session_text(header.agent_nickname, MAX_AGENT_IDENTITY_CHARS);
  let agent_role = normalize_session_text(header.agent_role, MAX_AGENT_IDENTITY_CHARS);
  Ok(SessionSummary {
    session_key: encode_session_key(&locator)?,
    session_id: header.id,
    provider,
    title,
    preview,
    project,
    cwd: header.cwd,
    updated_at_ms: header
      .updated_at_ms
      .or_else(|| parse_updated_at_ms(header.updated_at.as_deref())),
    timestamp: header.timestamp,
    parent_session_id: header.parent_session_id,
    is_subagent,
    agent_path,
    agent_nickname,
    agent_role,
    child_count,
    message_count: None,
    // The listing adapters deliberately inspect only headers. Loading every
    // normalized body here would make the bounded UI query unbounded.
    event_count: None,
    history_status: None,
  })
}

fn path_name(path: &str) -> Option<&str> {
  Path::new(path)
    .file_name()
    .and_then(|value| value.to_str())
    .filter(|value| !value.is_empty())
}

fn matches_search(session: &SessionSummary, search: Option<&str>) -> bool {
  let Some(search) = search else {
    return true;
  };
  [
    Some(session.session_id.as_str()),
    session.title.as_deref(),
    session.preview.as_deref(),
    session.project.as_deref(),
    session.cwd.as_deref(),
    session.agent_path.as_deref(),
    session.agent_nickname.as_deref(),
    session.agent_role.as_deref(),
  ]
  .into_iter()
  .flatten()
  .any(|value| value.to_lowercase().contains(search))
}

fn normalize_session_text(value: Option<String>, max_chars: usize) -> Option<String> {
  value
    .as_deref()
    .and_then(|value| normalize_one_line_text(value, max_chars))
}

fn normalize_one_line_text(value: &str, max_chars: usize) -> Option<String> {
  let mut normalized = String::with_capacity(value.len().min(max_chars.saturating_mul(4)));
  let mut characters = value.chars().peekable();
  let mut normalized_chars = 0;
  let mut needs_space = false;
  while let Some(character) = characters.next() {
    match character {
      '\u{001b}' => {
        match characters.next() {
          Some('[') => consume_control_sequence(&mut characters),
          Some(']') => consume_operating_system_command(&mut characters),
          Some(_) | None => {}
        }
        continue;
      }
      '\u{009b}' => {
        consume_control_sequence(&mut characters);
        continue;
      }
      _ => {}
    }
    if character.is_whitespace() {
      needs_space = !normalized.is_empty();
    } else if !is_unsafe_session_character(character) {
      if needs_space {
        if !push_bounded_character(&mut normalized, &mut normalized_chars, ' ', max_chars) {
          break;
        }
      }
      needs_space = false;
      if !push_bounded_character(&mut normalized, &mut normalized_chars, character, max_chars) {
        break;
      }
    }
  }
  if normalized.is_empty() { None } else { Some(normalized) }
}

fn push_bounded_character(output: &mut String, count: &mut usize, character: char, max_chars: usize) -> bool {
  if *count < max_chars {
    output.push(character);
    *count += 1;
    true
  } else {
    if max_chars > 0 {
      output.pop();
      output.push('\u{2026}');
    }
    false
  }
}

fn is_unsafe_session_character(character: char) -> bool {
  character.is_control()
    || matches!(
      character,
      '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'
    )
}

fn consume_control_sequence(characters: &mut std::iter::Peekable<std::str::Chars<'_>>) {
  for character in characters.by_ref() {
    if ('@'..='~').contains(&character) {
      break;
    }
  }
}

fn consume_operating_system_command(characters: &mut std::iter::Peekable<std::str::Chars<'_>>) {
  while let Some(character) = characters.next() {
    if character == '\u{0007}' {
      break;
    }
    if character == '\u{001b}' && characters.next_if_eq(&'\\').is_some() {
      break;
    }
  }
}

fn event_page_bounds(
  total: usize,
  boundary: usize,
  direction: PageDirection,
  limit: usize,
) -> Result<(usize, usize), String> {
  if boundary > total {
    return Err("event cursor is outside the session".to_string());
  }
  Ok(match direction {
    PageDirection::Forward => (boundary, boundary.saturating_add(limit).min(total)),
    PageDirection::Backward => (boundary.saturating_sub(limit), boundary),
  })
}

fn timeline_entries(events: &[AgentEvent]) -> Vec<TimelineEntry> {
  let mut operations_by_timeline_source = HashMap::new();
  let mut hidden_tool_sources = HashSet::new();

  for operation in assemble_tool_operations(events) {
    let Some(timeline_source_event_index) = operation.timeline_source_event_index() else {
      continue;
    };
    let Some(&source_event_index) = operation.source_event_indices.first() else {
      continue;
    };
    // Keep the invocation key stable for selection and cached detail, while
    // placing a terminal historical operation where its result occurred.
    hidden_tool_sources.extend(
      operation
        .source_event_indices
        .iter()
        .copied()
        .filter(|index| *index != timeline_source_event_index),
    );
    operations_by_timeline_source.insert(timeline_source_event_index, (source_event_index, operation));
  }

  let mut entries = Vec::with_capacity(events.len().saturating_sub(hidden_tool_sources.len()));
  for source_event_index in 0..events.len() {
    if let Some((detail_source_event_index, operation)) = operations_by_timeline_source.remove(&source_event_index) {
      entries.push(TimelineEntry::ToolOperation {
        source_event_index: detail_source_event_index,
        operation,
      });
    } else if hidden_tool_sources.contains(&source_event_index) {
      continue;
    } else {
      // This should only be reachable for non-tool records. Retaining a
      // standalone tool record if an assembler invariant is violated is safer
      // than silently hiding provider data.
      entries.push(TimelineEntry::Event { source_event_index });
    }
  }
  entries
}

fn timeline_entry_for_source(events: &[AgentEvent], source_event_index: usize) -> Option<TimelineEntry> {
  timeline_entries(events).into_iter().find(|entry| match entry {
    TimelineEntry::Event {
      source_event_index: entry_index,
    } => *entry_index == source_event_index,
    TimelineEntry::ToolOperation {
      source_event_index: entry_index,
      operation,
    } => *entry_index == source_event_index || operation.source_event_indices.contains(&source_event_index),
  })
}

fn event_summary(_events: &[AgentEvent], index: usize, event: &AgentEvent) -> EventSummary {
  let hidden = event.is_hidden();
  let title = if hidden {
    "Hidden provider content".to_string()
  } else {
    truncate(event_title(event), 120)
  };
  let reasoning = (!hidden).then(|| reasoning_card_summary(event)).flatten();
  let summary = if hidden {
    render_event_summary(event)
  } else {
    match event {
      AgentEvent::Message(message) => message.text.clone(),
      AgentEvent::Reasoning(_) => reasoning
        .as_ref()
        .and_then(|card| {
          card
            .is_redacted
            .then_some("Reasoning redacted by provider".to_string())
            .or_else(|| card.preview.clone())
        })
        .unwrap_or_else(|| "Reasoning".to_string()),
      _ => render_event_summary(event),
    }
  };
  let summary_max_chars = if !hidden && matches!(event, AgentEvent::Message(_)) {
    MAX_MESSAGE_SUMMARY_CHARS
  } else {
    MAX_TECHNICAL_SUMMARY_CHARS
  };
  let (summary, summary_truncated) = truncate_with_flag(summary, summary_max_chars);
  let tool = (!hidden).then(|| tool_event(event).map(tool_card_summary)).flatten();
  let usage = (!hidden).then(|| usage_card_summary(event)).flatten();
  EventSummary {
    event_key: encode_event_key(index),
    event_type: normalized_event_type(event).to_string(),
    provider: provider_for_event(event),
    timestamp: timestamp_for_event(event).map(str::to_string),
    phase: phase_for_event(event),
    role: role_for_event(event),
    title,
    summary,
    summary_truncated,
    is_hidden: hidden,
    is_error: error_for_event(event),
    tool,
    usage,
    reasoning,
  }
}

fn tool_operation_event_summary(source_event_index: usize, operation: &ToolOperation) -> EventSummary {
  let tool = tool_operation_card_summary(operation);
  let title = truncate(
    operation
      .tool_name
      .as_deref()
      .or(operation.provider_tool_name.as_deref())
      .unwrap_or("Tool operation")
      .to_string(),
    120,
  );
  let (summary, summary_truncated) =
    truncate_with_flag(tool_operation_summary(operation, &tool), MAX_TECHNICAL_SUMMARY_CHARS);
  EventSummary {
    event_key: encode_event_key(source_event_index),
    event_type: "tool_call".to_string(),
    provider: viewer_provider(operation.provider),
    timestamp: if operation.is_finished() {
      operation.updated_at.clone().or_else(|| operation.started_at.clone())
    } else {
      operation.started_at.clone().or_else(|| operation.updated_at.clone())
    },
    // A logical operation has an explicit derived status. Exposing the source
    // record phase here would recreate the old `finished` ambiguity.
    phase: None,
    role: None,
    title,
    summary,
    summary_truncated,
    is_hidden: false,
    // Preserve an unspecified provider error state. A failed assembled
    // operation is the only case where we need to synthesize `true`.
    is_error: operation
      .is_error
      .or(matches!(operation.status, ToolOperationStatus::Failed).then_some(true)),
    tool: Some(tool),
    usage: None,
    reasoning: None,
  }
}

fn tool_operation_summary(operation: &ToolOperation, card: &ToolCardSummary) -> String {
  match operation.summary.as_ref() {
    Some(ToolSummary::Shell { command, .. }) => command.clone().unwrap_or_else(|| "Shell operation".to_string()),
    Some(ToolSummary::Terminal {
      session_id,
      action,
      chars_len,
      wait_ms,
    }) => match action {
      Some(TerminalAction::Wait) => format!(
        "Wait for terminal {}{}",
        session_id.as_deref().unwrap_or("session"),
        wait_ms
          .map(|value| format!(" for up to {value} ms"))
          .unwrap_or_default(),
      ),
      Some(TerminalAction::Send) => format!(
        "Send {} characters to terminal {}",
        chars_len.unwrap_or(0),
        session_id.as_deref().unwrap_or("session"),
      ),
      None => "Terminal operation".to_string(),
    },
    Some(ToolSummary::CodeExecution { language }) => {
      format!("{} code execution", language.as_deref().unwrap_or("Unknown"))
    }
    Some(ToolSummary::FileRead { path }) => path.clone().unwrap_or_else(|| "Read file".to_string()),
    Some(ToolSummary::FileWrite { path, .. }) => path.clone().unwrap_or_else(|| "Write file".to_string()),
    Some(ToolSummary::FileEdit { path, .. }) => path.clone().unwrap_or_else(|| "Edit file".to_string()),
    Some(ToolSummary::Search { query }) => query.clone().unwrap_or_else(|| "Search".to_string()),
    Some(ToolSummary::Web { url }) => url.clone().unwrap_or_else(|| "Web request".to_string()),
    Some(ToolSummary::Task { title }) => title.clone().unwrap_or_else(|| "Task".to_string()),
    None => card.tool_name.clone().unwrap_or_else(|| "Tool operation".to_string()),
  }
}

fn usage_card_summary(event: &AgentEvent) -> Option<UsageCardSummary> {
  let AgentEvent::Usage(usage) = event else {
    return None;
  };

  Some(UsageCardSummary {
    kind: usage_kind_label(usage.kind).to_string(),
    input_tokens: usage.input_tokens.to_string(),
    output_tokens: usage.output_tokens.to_string(),
    total_tokens: usage.total_tokens.map(|value| value.to_string()),
    cache_read_tokens: usage.cache_read_tokens.map(|value| value.to_string()),
    cache_write_tokens: usage.cache_write_tokens.map(|value| value.to_string()),
    reasoning_tokens: usage.reasoning_tokens.map(|value| value.to_string()),
    turn_id: present_string(usage.turn_id.as_deref()).map(str::to_string),
    step_id: present_string(usage.step_id.as_deref()).map(str::to_string),
  })
}

fn usage_kind_label(kind: UsageKind) -> &'static str {
  match kind {
    UsageKind::ModelCall => "model_call",
    UsageKind::OperationTotal => "operation_total",
    UsageKind::SessionSnapshot => "session_snapshot",
  }
}

fn reasoning_card_summary(event: &AgentEvent) -> Option<ReasoningCardSummary> {
  let AgentEvent::Reasoning(reasoning) = event else {
    return None;
  };

  let summary_preview = reasoning
    .summary
    .as_deref()
    .and_then(|value| normalize_one_line_text(value, MAX_REASONING_CARD_PREVIEW_CHARS));
  let text_preview = reasoning
    .text
    .as_deref()
    .and_then(|value| normalize_one_line_text(value, MAX_REASONING_CARD_PREVIEW_CHARS));
  let is_redacted = reasoning.redacted == Some(true);
  let has_summary = reasoning.summary.is_some();
  let has_text = reasoning.text.is_some();
  let preview = (!is_redacted).then(|| summary_preview.or(text_preview)).flatten();

  Some(ReasoningCardSummary {
    preview,
    has_summary,
    has_text,
    has_encrypted_content: reasoning.encrypted_content.is_some(),
    is_redacted,
  })
}

fn tool_card_summary(event: &ToolCallEvent) -> ToolCardSummary {
  let mut card = ToolCardSummary {
    kind: tool_kind_label(event.tool_kind).to_string(),
    tool_name: present_string(event.tool_name.as_deref()).map(str::to_string),
    tool_call_id: present_string(event.tool_call_id.as_deref()).map(str::to_string),
    status: tool_record_status_label(event).to_string(),
    provider_tool_name: present_string(event.effective_provider_tool_name()).map(str::to_string),
    language: None,
    command: None,
    cwd: None,
    terminal_session_id: None,
    terminal_action: None,
    chars_len: None,
    wait_ms: None,
    path: None,
    query: None,
    url: None,
    task_title: None,
    exit_code: None,
    bytes: None,
    added: None,
    removed: None,
  };

  match event.summary.as_ref() {
    Some(ToolSummary::CodeExecution { language }) => card.language.clone_from(language),
    Some(ToolSummary::Shell {
      command,
      cwd,
      exit_code,
    }) => {
      card.command.clone_from(command);
      card.cwd.clone_from(cwd);
      card.exit_code = *exit_code;
    }
    Some(ToolSummary::Terminal {
      session_id,
      action,
      chars_len,
      wait_ms,
    }) => {
      card.terminal_session_id.clone_from(session_id);
      card.terminal_action = action.map(terminal_action_label).map(str::to_string);
      card.chars_len = *chars_len;
      card.wait_ms = *wait_ms;
    }
    Some(ToolSummary::FileRead { path }) => card.path.clone_from(path),
    Some(ToolSummary::FileWrite { path, bytes }) => {
      card.path.clone_from(path);
      card.bytes = *bytes;
    }
    Some(ToolSummary::FileEdit { path, added, removed }) => {
      card.path.clone_from(path);
      card.added = *added;
      card.removed = *removed;
    }
    Some(ToolSummary::Search { query }) => card.query.clone_from(query),
    Some(ToolSummary::Web { url }) => card.url.clone_from(url),
    Some(ToolSummary::Task { title }) => card.task_title.clone_from(title),
    None => {}
  }
  card
}

fn tool_operation_card_summary(operation: &ToolOperation) -> ToolCardSummary {
  let mut card = ToolCardSummary {
    kind: tool_kind_label(operation.tool_kind).to_string(),
    tool_name: present_string(operation.tool_name.as_deref()).map(str::to_string),
    tool_call_id: present_string(operation.tool_call_id.as_deref()).map(str::to_string),
    status: tool_operation_status_label(operation.status).to_string(),
    provider_tool_name: present_string(operation.provider_tool_name.as_deref()).map(str::to_string),
    language: None,
    command: None,
    cwd: None,
    terminal_session_id: None,
    terminal_action: None,
    chars_len: None,
    wait_ms: None,
    path: None,
    query: None,
    url: None,
    task_title: None,
    exit_code: None,
    bytes: None,
    added: None,
    removed: None,
  };

  match operation.summary.as_ref() {
    Some(ToolSummary::CodeExecution { language }) => card.language.clone_from(language),
    Some(ToolSummary::Shell {
      command,
      cwd,
      exit_code,
    }) => {
      card.command.clone_from(command);
      card.cwd.clone_from(cwd);
      card.exit_code = *exit_code;
    }
    Some(ToolSummary::Terminal {
      session_id,
      action,
      chars_len,
      wait_ms,
    }) => {
      card.terminal_session_id.clone_from(session_id);
      card.terminal_action = action.map(terminal_action_label).map(str::to_string);
      card.chars_len = *chars_len;
      card.wait_ms = *wait_ms;
    }
    Some(ToolSummary::FileRead { path }) => card.path.clone_from(path),
    Some(ToolSummary::FileWrite { path, bytes }) => {
      card.path.clone_from(path);
      card.bytes = *bytes;
    }
    Some(ToolSummary::FileEdit { path, added, removed }) => {
      card.path.clone_from(path);
      card.added = *added;
      card.removed = *removed;
    }
    Some(ToolSummary::Search { query }) => card.query.clone_from(query),
    Some(ToolSummary::Web { url }) => card.url.clone_from(url),
    Some(ToolSummary::Task { title }) => card.task_title.clone_from(title),
    None => {}
  }
  bound_tool_card_summary(card)
}

fn terminal_action_label(action: TerminalAction) -> &'static str {
  match action {
    TerminalAction::Send => "send",
    TerminalAction::Wait => "wait",
  }
}

fn tool_operation_status_label(status: ToolOperationStatus) -> &'static str {
  match status {
    ToolOperationStatus::Pending => "pending",
    ToolOperationStatus::Running => "running",
    ToolOperationStatus::Completed => "completed",
    ToolOperationStatus::Failed => "failed",
  }
}

fn tool_record_status_label(event: &ToolCallEvent) -> &'static str {
  if event.is_error == Some(true) {
    return "failed";
  }
  match event.phase {
    Phase::Started => "pending",
    Phase::Delta | Phase::Updated => "running",
    Phase::Finished => "completed",
  }
}

fn bound_tool_card_summary(mut card: ToolCardSummary) -> ToolCardSummary {
  card.kind = truncate(card.kind, MAX_TOOL_NAME_CHARS);
  card.tool_name = truncate_option(card.tool_name, MAX_TOOL_NAME_CHARS);
  card.tool_call_id = truncate_option(card.tool_call_id, MAX_TOOL_CARD_STRING_CHARS);
  card.status = truncate(card.status, MAX_TOOL_NAME_CHARS);
  card.provider_tool_name = truncate_option(card.provider_tool_name, MAX_TOOL_NAME_CHARS);
  card.language = truncate_option(card.language, MAX_TOOL_NAME_CHARS);
  card.command = truncate_option(card.command, MAX_TOOL_CARD_STRING_CHARS);
  card.cwd = truncate_option(card.cwd, MAX_TOOL_CARD_STRING_CHARS);
  card.terminal_session_id = truncate_option(card.terminal_session_id, MAX_TOOL_CARD_STRING_CHARS);
  card.terminal_action = truncate_option(card.terminal_action, MAX_TOOL_NAME_CHARS);
  card.path = truncate_option(card.path, MAX_TOOL_CARD_STRING_CHARS);
  card.query = truncate_option(card.query, MAX_TOOL_CARD_STRING_CHARS);
  card.url = truncate_option(card.url, MAX_TOOL_CARD_STRING_CHARS);
  card.task_title = truncate_option(card.task_title, MAX_TOOL_CARD_STRING_CHARS);
  card
}

fn truncate_option(value: Option<String>, max_chars: usize) -> Option<String> {
  value.map(|value| truncate(value, max_chars))
}

fn tool_kind_label(kind: ToolKind) -> &'static str {
  match kind {
    ToolKind::CodeExecution => "code_execution",
    ToolKind::Shell => "shell",
    ToolKind::Terminal => "terminal",
    ToolKind::FileRead => "file_read",
    ToolKind::FileWrite => "file_write",
    ToolKind::FileEdit => "file_edit",
    ToolKind::Search => "search",
    ToolKind::Web => "web",
    ToolKind::Task => "task",
    ToolKind::Unknown => "unknown",
  }
}

/// Matches the serialized `AgentEvent` discriminants consumed by the viewer.
/// Keep this exhaustive so adding an IR variant cannot silently fall back to a
/// render-layer display label.
fn normalized_event_type(event: &AgentEvent) -> &'static str {
  match event {
    AgentEvent::SessionStarted(_) => "session_started",
    AgentEvent::ProviderChanged(_) => "provider_changed",
    AgentEvent::SessionSettingsApplied(_) => "session_settings_applied",
    AgentEvent::Message(_) => "message",
    AgentEvent::Reasoning(_) => "reasoning",
    AgentEvent::GoalUpdated(_) => "goal_updated",
    AgentEvent::AgentActivity(_) => "agent_activity",
    AgentEvent::ToolCall(_) => "tool_call",
    AgentEvent::Lifecycle(_) => "lifecycle",
    AgentEvent::Usage(_) => "usage",
    AgentEvent::Metadata(_) => "metadata",
    AgentEvent::Error(_) => "error",
    AgentEvent::Unknown(_) => "unknown",
  }
}

fn event_title(event: &AgentEvent) -> String {
  match event {
    AgentEvent::SessionStarted(_) => "Session started".to_string(),
    AgentEvent::ProviderChanged(_) => "Provider changed".to_string(),
    AgentEvent::SessionSettingsApplied(_) => "Session settings".to_string(),
    AgentEvent::Message(event) => match event.role {
      Role::User => "User".to_string(),
      Role::Assistant => "Assistant".to_string(),
      Role::System => "System".to_string(),
      Role::Tool => "Tool message".to_string(),
      Role::Unknown => "Message".to_string(),
    },
    AgentEvent::Reasoning(_) => "Reasoning".to_string(),
    AgentEvent::GoalUpdated(_) => "Goal updated".to_string(),
    AgentEvent::AgentActivity(_) => "Agent activity".to_string(),
    AgentEvent::ToolCall(event) => event.tool_name.clone().unwrap_or_else(|| "Tool call".to_string()),
    AgentEvent::Lifecycle(_) => "Lifecycle".to_string(),
    AgentEvent::Usage(_) => "Usage".to_string(),
    AgentEvent::Metadata(event) => event.native_type.clone(),
    AgentEvent::Error(_) => "Error".to_string(),
    AgentEvent::Unknown(event) => event.native_type.clone().unwrap_or_else(|| "Unknown event".to_string()),
  }
}

fn provider_for_event(event: &AgentEvent) -> ViewerProvider {
  let provider = match event {
    AgentEvent::SessionStarted(event) => event.provider,
    AgentEvent::ProviderChanged(event) => event.provider,
    AgentEvent::SessionSettingsApplied(event) => event.provider,
    AgentEvent::Message(event) => event.provider,
    AgentEvent::Reasoning(event) => event.provider,
    AgentEvent::GoalUpdated(event) => event.provider,
    AgentEvent::AgentActivity(event) => event.provider,
    AgentEvent::ToolCall(event) => event.provider,
    AgentEvent::Lifecycle(event) => event.provider,
    AgentEvent::Usage(event) => event.provider,
    AgentEvent::Metadata(event) => event.provider,
    AgentEvent::Error(event) => event.provider,
    AgentEvent::Unknown(event) => event.provider,
  };
  viewer_provider(provider)
}

fn viewer_provider(provider: Provider) -> ViewerProvider {
  match provider {
    Provider::Codex => ViewerProvider::Codex,
    Provider::Pi => ViewerProvider::Pi,
    Provider::OpenCode => ViewerProvider::OpenCode,
    Provider::Dsh => ViewerProvider::Dsh,
  }
}

fn timestamp_for_event(event: &AgentEvent) -> Option<&str> {
  match event {
    AgentEvent::SessionStarted(event) => event.timestamp.as_deref(),
    AgentEvent::ProviderChanged(event) => event.timestamp.as_deref(),
    AgentEvent::SessionSettingsApplied(event) => event.timestamp.as_deref(),
    AgentEvent::Message(event) => event.timestamp.as_deref(),
    AgentEvent::Reasoning(event) => event.timestamp.as_deref(),
    AgentEvent::GoalUpdated(event) => event.timestamp.as_deref(),
    AgentEvent::AgentActivity(event) => event.timestamp.as_deref(),
    AgentEvent::ToolCall(event) => event.timestamp.as_deref(),
    AgentEvent::Lifecycle(event) => event.timestamp.as_deref(),
    AgentEvent::Usage(event) => event.timestamp.as_deref(),
    AgentEvent::Metadata(event) => event.timestamp.as_deref(),
    AgentEvent::Error(event) => event.timestamp.as_deref(),
    AgentEvent::Unknown(event) => event.timestamp.as_deref(),
  }
}

fn phase_for_event(event: &AgentEvent) -> Option<String> {
  match event {
    AgentEvent::Message(event) => serialized_label(event.phase),
    AgentEvent::Reasoning(event) => serialized_label(event.phase),
    AgentEvent::ToolCall(event) => serialized_label(event.phase),
    AgentEvent::Lifecycle(event) => serialized_label(event.phase),
    _ => None,
  }
}

fn role_for_event(event: &AgentEvent) -> Option<String> {
  match event {
    AgentEvent::Message(event) => serialized_label(event.role),
    _ => None,
  }
}

fn serialized_label(value: impl Serialize) -> Option<String> {
  serde_json::to_value(value).ok()?.as_str().map(str::to_string)
}

fn error_for_event(event: &AgentEvent) -> Option<bool> {
  match event {
    AgentEvent::Error(_) => Some(true),
    AgentEvent::ToolCall(event) => event.is_error,
    AgentEvent::Lifecycle(event) => Some(matches!(event.outcome, Some(LifecycleOutcome::Failed))),
    _ => None,
  }
}

fn tool_event(event: &AgentEvent) -> Option<&ToolCallEvent> {
  match event {
    AgentEvent::ToolCall(event) => Some(event),
    _ => None,
  }
}

fn present_string(value: Option<&str>) -> Option<&str> {
  value.map(str::trim).filter(|value| !value.is_empty())
}

fn tool_output_preview(events: &[AgentEvent], index: usize) -> Option<ToolOutputPreview> {
  let origin = events.get(index).and_then(tool_event)?;
  projected_event_output(origin).map(|sections| bound_tool_output(sections, index))
}

fn projected_event_output(event: &ToolCallEvent) -> Option<Vec<ToolOutputSection>> {
  let output = event.output.as_ref().filter(|value| !value.is_null())?;
  let sections = project_output(output, 0);
  (!sections.is_empty()).then_some(sections)
}

fn project_output(value: &Value, depth: usize) -> Vec<ToolOutputSection> {
  const MAX_OUTPUT_PROJECTION_DEPTH: usize = 16;
  if depth >= MAX_OUTPUT_PROJECTION_DEPTH {
    return json_section(None, value).into_iter().collect();
  }

  match value {
    Value::Null => Vec::new(),
    Value::String(text) if text.is_empty() => Vec::new(),
    Value::String(text) => vec![text_section(None, text.clone())],
    Value::Array(values) if values.is_empty() => Vec::new(),
    Value::Array(_) => content_text(value)
      .map(|text| vec![text_section(None, text)])
      .unwrap_or_else(|| json_section(None, value).into_iter().collect()),
    Value::Object(object) if object.is_empty() => Vec::new(),
    Value::Object(object) => {
      if let Some(sections) = shell_output_sections(object) {
        return sections;
      }

      // OpenCode wraps provider output in an `output` object. Recurse through
      // that wrapper before considering metadata or the raw fallback.
      if object.contains_key("output") {
        let mut sections = object
          .get("output")
          .map(|output| project_output(output, depth + 1))
          .unwrap_or_default();
        if sections.is_empty()
          && let Some(raw) = object.get("raw")
        {
          sections = project_labeled_value("Raw", raw, depth + 1);
        }
        if sections.is_empty()
          && let Some(error) = object.get("error")
        {
          sections = project_labeled_value("Error", error, depth + 1);
        }
        if sections.is_empty()
          && let Some(metadata) = object.get("metadata").filter(|value| !is_effectively_empty(value))
        {
          sections = project_labeled_value("Metadata", metadata, depth + 1);
        }
        return sections;
      }

      // Semantic tool adapters, including Codex Code Mode, commonly retain
      // response metadata next to one readable `text` field. Show that
      // payload directly instead of turning the entire result into JSON.
      if let Some(text) = object
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
      {
        let mut sections = vec![text_section(None, text.to_string())];
        append_distinct_error(&mut sections, object.get("error"), depth + 1);
        return sections;
      }

      if let Some(content) = object.get("content") {
        let mut sections = content_text(content)
          .map(|text| vec![text_section(None, text)])
          .unwrap_or_else(|| project_output(content, depth + 1));
        append_distinct_error(&mut sections, object.get("error"), depth + 1);
        if !sections.is_empty() {
          return sections;
        }
      }

      if let Some(content_items) = object.get("content_items") {
        let mut sections = content_text(content_items)
          .map(|text| vec![text_section(None, text)])
          .unwrap_or_else(|| project_labeled_value("Content", content_items, depth + 1));
        append_distinct_error(&mut sections, object.get("error"), depth + 1);
        if !sections.is_empty() {
          return sections;
        }
      }

      if let Some(result) = object.get("result") {
        let sections = project_labeled_value("Result", result, depth + 1);
        if !sections.is_empty() {
          return sections;
        }
      }

      if let Some(results) = object.get("results") {
        let sections = project_labeled_value("Results", results, depth + 1);
        if !sections.is_empty() {
          return sections;
        }
      }

      if let Some(error) = object.get("error") {
        let sections = project_labeled_value("Error", error, depth + 1);
        if !sections.is_empty() {
          return sections;
        }
      }

      if is_effectively_empty(value) {
        Vec::new()
      } else {
        json_section(None, value).into_iter().collect()
      }
    }
    Value::Bool(_) | Value::Number(_) => json_section(None, value).into_iter().collect(),
  }
}

fn shell_output_sections(object: &serde_json::Map<String, Value>) -> Option<Vec<ToolOutputSection>> {
  const SHELL_FIELDS: [&str; 4] = ["formatted_output", "aggregated_output", "stdout", "stderr"];
  if !SHELL_FIELDS.iter().any(|field| object.contains_key(*field)) {
    return None;
  }

  let main = ["formatted_output", "aggregated_output", "stdout"]
    .into_iter()
    .find_map(|field| nonempty_string_field(object, field).map(|text| (field, text)));
  let stderr = nonempty_string_field(object, "stderr");
  let mut sections = Vec::new();
  if let Some((field, text)) = main {
    let label = if field == "stdout" { "Stdout" } else { "Output" };
    sections.push(text_section(Some(label), text.to_string()));
  }
  if let Some(stderr) = stderr {
    let is_distinct = sections
      .iter()
      .all(|section| section.text != stderr && !section.text.contains(stderr));
    if is_distinct {
      sections.push(text_section(Some("Stderr"), stderr.to_string()));
    }
  }

  if sections.is_empty()
    && SHELL_FIELDS
      .iter()
      .filter_map(|field| object.get(*field))
      .any(|value| !is_effectively_empty(value))
  {
    return json_section(None, &Value::Object(object.clone()))
      .map(|section| vec![section])
      .or_else(|| Some(Vec::new()));
  }
  Some(sections)
}

fn nonempty_string_field<'a>(object: &'a serde_json::Map<String, Value>, field: &str) -> Option<&'a str> {
  object
    .get(field)
    .and_then(Value::as_str)
    .filter(|text| !text.is_empty())
}

fn project_labeled_value(label: &str, value: &Value, depth: usize) -> Vec<ToolOutputSection> {
  let mut sections = project_output(value, depth);
  if sections.len() == 1 && sections[0].label.is_none() {
    sections[0].label = Some(label.to_string());
  }
  sections
}

fn append_distinct_error(sections: &mut Vec<ToolOutputSection>, error: Option<&Value>, depth: usize) {
  let Some(error) = error.filter(|value| !is_effectively_empty(value)) else {
    return;
  };
  for section in project_labeled_value("Error", error, depth) {
    let is_distinct = sections
      .iter()
      .all(|existing| existing.text != section.text && !existing.text.contains(&section.text));
    if is_distinct {
      sections.push(section);
    }
  }
}

fn content_text(value: &Value) -> Option<String> {
  let mut parts = Vec::new();
  collect_content_text(value, 0, &mut parts);
  (!parts.is_empty()).then(|| parts.join("\n"))
}

fn collect_content_text(value: &Value, depth: usize, parts: &mut Vec<String>) {
  const MAX_CONTENT_DEPTH: usize = 16;
  if depth >= MAX_CONTENT_DEPTH {
    return;
  }
  match value {
    Value::String(text) if !text.is_empty() => parts.push(text.clone()),
    Value::Array(values) => {
      for value in values {
        collect_content_text(value, depth + 1, parts);
      }
    }
    Value::Object(object) => {
      if let Some(text) = object
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
      {
        parts.push(text.to_string());
      } else if let Some(content) = object.get("content") {
        collect_content_text(content, depth + 1, parts);
      }
    }
    Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
  }
}

fn is_effectively_empty(value: &Value) -> bool {
  match value {
    Value::Null => true,
    Value::String(text) => text.is_empty(),
    Value::Array(values) => values.iter().all(is_effectively_empty),
    Value::Object(object) => object.values().all(is_effectively_empty),
    Value::Bool(_) | Value::Number(_) => false,
  }
}

fn text_section(label: Option<&str>, text: String) -> ToolOutputSection {
  ToolOutputSection {
    label: label.map(str::to_string),
    text,
    format: "text".to_string(),
  }
}

fn json_section(label: Option<&str>, value: &Value) -> Option<ToolOutputSection> {
  if is_effectively_empty(value) {
    return None;
  }
  serde_json::to_string_pretty(value).ok().map(|text| ToolOutputSection {
    label: label.map(str::to_string),
    text,
    format: "json".to_string(),
  })
}

fn bound_tool_output(mut sections: Vec<ToolOutputSection>, source_index: usize) -> ToolOutputPreview {
  let original_size_bytes = sections
    .iter()
    .fold(0usize, |total, section| total.saturating_add(section.text.len()));
  let truncated = original_size_bytes > MAX_TOOL_OUTPUT_BYTES;
  if truncated {
    let budgets = section_budgets(&sections, MAX_TOOL_OUTPUT_BYTES);
    for (section, budget) in sections.iter_mut().zip(budgets) {
      section.text = truncate_utf8_head_tail(&section.text, budget);
    }
    sections.retain(|section| !section.text.is_empty());
  }
  ToolOutputPreview {
    sections,
    truncated,
    original_size_bytes,
    source_event_key: encode_event_key(source_index),
  }
}

fn section_budgets(sections: &[ToolOutputSection], limit: usize) -> Vec<usize> {
  let mut budgets = vec![0; sections.len()];
  let mut pending: Vec<usize> = (0..sections.len()).collect();
  let mut remaining = limit;

  while !pending.is_empty() {
    let fair_share = remaining / pending.len();
    let small: Vec<usize> = pending
      .iter()
      .copied()
      .filter(|index| sections[*index].text.len() <= fair_share)
      .collect();
    if small.is_empty() {
      for (position, index) in pending.iter().copied().enumerate() {
        let budget = fair_share + usize::from(position < remaining % pending.len());
        budgets[index] = budget;
      }
      break;
    }
    for index in &small {
      let size = sections[*index].text.len();
      budgets[*index] = size;
      remaining = remaining.saturating_sub(size);
    }
    pending.retain(|index| !small.contains(index));
  }
  budgets
}

fn truncate_utf8_head_tail(text: &str, limit: usize) -> String {
  if text.len() <= limit {
    return text.to_string();
  }
  if limit <= TOOL_OUTPUT_TRUNCATION_MARKER.len() {
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
      end -= 1;
    }
    return text[..end].to_string();
  }

  let retained = limit - TOOL_OUTPUT_TRUNCATION_MARKER.len();
  let requested_head = retained.div_ceil(2);
  let requested_tail = retained / 2;
  let mut head_end = requested_head;
  while head_end > 0 && !text.is_char_boundary(head_end) {
    head_end -= 1;
  }
  let mut tail_start = text.len().saturating_sub(requested_tail);
  while tail_start < text.len() && !text.is_char_boundary(tail_start) {
    tail_start += 1;
  }
  format!(
    "{}{}{}",
    &text[..head_end],
    TOOL_OUTPUT_TRUNCATION_MARKER,
    &text[tail_start..]
  )
}

fn native_detail(event: &AgentEvent) -> Option<Value> {
  match event {
    AgentEvent::SessionSettingsApplied(event) => event.native.clone(),
    AgentEvent::Message(event) => event.provenance.as_ref().and_then(|value| value.native.clone()),
    AgentEvent::Reasoning(event) => event.provenance.as_ref().and_then(|value| value.native.clone()),
    AgentEvent::AgentActivity(event) => event.native.clone(),
    AgentEvent::Lifecycle(event) => Some(event.native.clone()),
    AgentEvent::Usage(event) => Some(event.native.clone()),
    AgentEvent::Metadata(event) => Some(event.native.clone()),
    AgentEvent::Unknown(event) => event.native.clone(),
    AgentEvent::SessionStarted(_)
    | AgentEvent::ProviderChanged(_)
    | AgentEvent::GoalUpdated(_)
    | AgentEvent::ToolCall(_)
    | AgentEvent::Error(_) => None,
  }
}

fn remove_embedded_native(event: &mut Value) {
  let Some(event) = event.as_object_mut() else {
    return;
  };
  event.remove("native");
  if let Some(provenance) = event.get_mut("provenance").and_then(Value::as_object_mut) {
    provenance.remove("native");
  }
}

fn bounded_detail_value(value: Value, representation: &'static str) -> Result<Value, String> {
  let original_size_bytes = serde_json::to_vec(&value)
    .map_err(|error| format!("failed to size {representation} detail: {error}"))?
    .len();
  if original_size_bytes <= MAX_DETAIL_VALUE_BYTES {
    return Ok(value);
  }
  Ok(json!({
    "truncated": true,
    "original_size_bytes": original_size_bytes,
    "limit_bytes": MAX_DETAIL_VALUE_BYTES,
    "representation": representation,
  }))
}

fn truncate(value: String, max_chars: usize) -> String {
  truncate_with_flag(value, max_chars).0
}

fn truncate_with_flag(value: String, max_chars: usize) -> (String, bool) {
  if value.chars().count() <= max_chars {
    return (value, false);
  }
  let mut truncated: String = value.chars().take(max_chars.saturating_sub(1)).collect();
  truncated.push('…');
  (truncated, true)
}

#[cfg(test)]
mod tests {
  use std::collections::HashMap;
  use std::path::PathBuf;
  use std::sync::atomic::{AtomicUsize, Ordering};
  use std::sync::{Barrier, Mutex};

  use serde_json::json;
  use tokn_session_core::{
    ErrorEvent, MessageDelivery, MessageEvent, MessageProvenance, Phase, ReasoningEvent, SessionHistoryStatus,
    SessionRef, ToolCallEvent, ToolKind, ToolRecordKind, ToolTransport, UnknownEvent, UsageEvent, UsageKind,
  };

  use super::*;
  use crate::model::{SessionQuery, decode_event_cursor, decode_session_key};

  struct FakeRepository {
    listings: HashMap<ViewerProvider, Result<Vec<SessionHeader>, String>>,
    loaded: Mutex<Option<LoadedSession>>,
  }

  impl ViewerRepository for FakeRepository {
    fn list_session_headers(&self, provider: ViewerProvider) -> Result<Vec<SessionHeader>, String> {
      self.listings.get(&provider).cloned().unwrap_or_else(|| Ok(Vec::new()))
    }

    fn hydrate_session_header(
      &self,
      _provider: ViewerProvider,
      header: SessionHeader,
    ) -> Result<SessionHeader, String> {
      Ok(header)
    }

    fn load_session(&self, _locator: &SessionLocator) -> Result<LoadedSession, String> {
      self
        .loaded
        .lock()
        .expect("fixture lock should not be poisoned")
        .take()
        .ok_or_else(|| "fixture session already loaded".to_string())
    }
  }

  struct CountingRepository {
    loads: Arc<AtomicUsize>,
  }

  impl ViewerRepository for CountingRepository {
    fn list_session_headers(&self, _provider: ViewerProvider) -> Result<Vec<SessionHeader>, String> {
      Ok(Vec::new())
    }

    fn hydrate_session_header(
      &self,
      _provider: ViewerProvider,
      header: SessionHeader,
    ) -> Result<SessionHeader, String> {
      Ok(header)
    }

    fn load_session(&self, _locator: &SessionLocator) -> Result<LoadedSession, String> {
      self.loads.fetch_add(1, Ordering::SeqCst);
      Ok(visible_session(1))
    }
  }

  struct HydratingRepository {
    headers: Vec<SessionHeader>,
    hydrations: Arc<AtomicUsize>,
  }

  struct FlakyHydratingRepository {
    header: SessionHeader,
    attempts: Arc<AtomicUsize>,
  }

  struct SlowHydratingRepository {
    header: SessionHeader,
    attempts: Arc<AtomicUsize>,
  }

  impl ViewerRepository for SlowHydratingRepository {
    fn list_session_headers(&self, provider: ViewerProvider) -> Result<Vec<SessionHeader>, String> {
      Ok(if provider == ViewerProvider::Dsh {
        vec![self.header.clone()]
      } else {
        Vec::new()
      })
    }

    fn hydrate_session_header(
      &self,
      _provider: ViewerProvider,
      mut header: SessionHeader,
    ) -> Result<SessionHeader, String> {
      self.attempts.fetch_add(1, Ordering::SeqCst);
      std::thread::sleep(std::time::Duration::from_millis(40));
      header.preview = Some("Shared prompt".to_string());
      Ok(header)
    }

    fn load_session(&self, _locator: &SessionLocator) -> Result<LoadedSession, String> {
      Err("not used by this fixture".to_string())
    }
  }

  impl ViewerRepository for FlakyHydratingRepository {
    fn list_session_headers(&self, provider: ViewerProvider) -> Result<Vec<SessionHeader>, String> {
      Ok(if provider == ViewerProvider::Pi {
        vec![self.header.clone()]
      } else {
        Vec::new()
      })
    }

    fn hydrate_session_header(
      &self,
      _provider: ViewerProvider,
      mut header: SessionHeader,
    ) -> Result<SessionHeader, String> {
      if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
        return Err("transient read failure".to_string());
      }
      header.preview = Some("Recovered prompt".to_string());
      Ok(header)
    }

    fn load_session(&self, _locator: &SessionLocator) -> Result<LoadedSession, String> {
      Err("not used by this fixture".to_string())
    }
  }

  impl ViewerRepository for HydratingRepository {
    fn list_session_headers(&self, provider: ViewerProvider) -> Result<Vec<SessionHeader>, String> {
      Ok(if provider == ViewerProvider::Codex {
        self.headers.clone()
      } else {
        Vec::new()
      })
    }

    fn hydrate_session_header(
      &self,
      _provider: ViewerProvider,
      mut header: SessionHeader,
    ) -> Result<SessionHeader, String> {
      self.hydrations.fetch_add(1, Ordering::SeqCst);
      header.preview = Some(format!("Prompt for {}", header.id));
      Ok(header)
    }

    fn load_session(&self, _locator: &SessionLocator) -> Result<LoadedSession, String> {
      Err("not used by this fixture".to_string())
    }
  }

  #[test]
  fn listing_filters_roots_searches_paginates_and_isolates_provider_errors() {
    let listings = HashMap::from([
      (
        ViewerProvider::Codex,
        Ok(vec![
          session_header("root-new", None, "/projects/Alpha", "2026-06-05T00:00:00Z"),
          session_header(
            "child-hidden",
            Some("root-new"),
            "/projects/Alpha",
            "2026-06-06T00:00:00Z",
          ),
          session_header("root-old", None, "/projects/Beta", "2026-06-01T00:00:00Z"),
        ]),
      ),
      (
        ViewerProvider::Pi,
        Ok(vec![session_header("pi-root", None, "/projects/Alpha", "1000")]),
      ),
      (ViewerProvider::Dsh, Err("fixture provider unavailable".to_string())),
    ]);
    let service = ViewerService::new(Arc::new(FakeRepository {
      listings,
      loaded: Mutex::new(None),
    }));

    let first = service
      .list_sessions(ListSessionsRequest {
        query: SessionQuery {
          providers: Vec::new(),
          search: Some("alpha".to_string()),
        },
        cursor: None,
        offset: None,
        limit: Some(1),
      })
      .unwrap();

    assert_eq!(first.sessions.len(), 1);
    assert_eq!(first.sessions[0].session_id, "root-new");
    assert_eq!(first.sessions[0].message_count, None);
    assert!(serde_json::to_value(&first.sessions[0]).unwrap()["message_count"].is_null());
    assert!(first.sessions.iter().all(|session| session.parent_session_id.is_none()));
    assert_eq!(first.source_errors.len(), 1);
    assert_eq!(first.source_errors[0].provider, ViewerProvider::Dsh);
    let next_cursor = first.next_cursor.expect("a second root matches");

    let second = service
      .list_sessions(ListSessionsRequest {
        query: SessionQuery {
          providers: Vec::new(),
          search: Some("alpha".to_string()),
        },
        cursor: Some(next_cursor),
        offset: None,
        limit: Some(1),
      })
      .unwrap();
    assert_eq!(second.sessions[0].session_id, "pi-root");
    assert!(second.next_cursor.is_none());
  }

  #[test]
  fn child_listing_is_paged_uses_provider_timestamp_and_keeps_agent_identity_safe() {
    let mut newest_child = session_header("child", Some("root"), "/projects/Alpha", "3000");
    newest_child.path = PathBuf::from("/fixtures/child-new.jsonl");
    // Filesystem times can be rewritten by a sync or restore. The source's
    // provider timestamp, not mtime, decides which duplicate owns this ID.
    newest_child.updated_at = Some("1".to_string());
    newest_child.updated_at_ms = Some(1);
    newest_child.agent_path = Some(" /root/\u{001b}[31mresearcher\u{202e} ".to_string());
    newest_child.agent_nickname = Some(" Hubble\n\t".to_string());
    newest_child.agent_role = Some(" explorer ".to_string());

    let mut older_duplicate = session_header("child", Some("root"), "/projects/Alpha", "2000");
    older_duplicate.path = PathBuf::from("/fixtures/child-old.jsonl");
    older_duplicate.updated_at = Some("9999".to_string());
    older_duplicate.updated_at_ms = Some(9_999);
    older_duplicate.agent_nickname = Some("older duplicate".to_string());

    let mut pi_same_id = session_header("child", Some("root"), "/projects/Pi", "4000");
    pi_same_id.path = PathBuf::from("/fixtures/pi-child.jsonl");
    let service = ViewerService::new(Arc::new(FakeRepository {
      listings: HashMap::from([
        (
          ViewerProvider::Codex,
          Ok(vec![
            session_header("root", None, "/projects/Alpha", "1000"),
            older_duplicate,
            newest_child,
            session_header("grandchild", Some("child"), "/projects/Alpha", "2500"),
            session_header("sibling", Some("root"), "/projects/Alpha", "1500"),
          ]),
        ),
        (ViewerProvider::Pi, Ok(vec![pi_same_id])),
      ]),
      loaded: Mutex::new(None),
    }));

    let roots = service
      .list_sessions(ListSessionsRequest {
        query: SessionQuery {
          providers: vec![ViewerProvider::Codex],
          search: None,
        },
        cursor: None,
        offset: None,
        limit: None,
      })
      .unwrap();
    assert_eq!(roots.sessions.len(), 1);
    assert_eq!(roots.sessions[0].session_id, "root");
    assert!(!roots.sessions[0].is_subagent);
    assert_eq!(roots.sessions[0].child_count, 2);

    let children = service
      .list_session_children(ListSessionChildrenRequest {
        parent_session_key: roots.sessions[0].session_key.clone(),
        cursor: None,
        offset: None,
        limit: Some(1),
      })
      .unwrap();
    assert_eq!(children.sessions.len(), 1);
    let child_cursor = children.next_cursor.clone().expect("a second direct child is paged");
    // Last-update time controls the visible order, independently from
    // provider-time canonicalization of duplicate identities.
    assert_eq!(children.sessions[0].session_id, "sibling");

    let second_child_page = service
      .list_session_children(ListSessionChildrenRequest {
        parent_session_key: roots.sessions[0].session_key.clone(),
        cursor: Some(child_cursor),
        offset: None,
        limit: Some(1),
      })
      .unwrap();
    assert_eq!(second_child_page.sessions.len(), 1);
    let child = &second_child_page.sessions[0];
    assert_eq!(child.session_id, "child");
    assert!(child.is_subagent);
    assert_eq!(child.child_count, 1);
    assert_eq!(child.agent_path.as_deref(), Some("/root/researcher"));
    assert_eq!(child.agent_nickname.as_deref(), Some("Hubble"));
    assert_eq!(child.agent_role.as_deref(), Some("explorer"));
    assert!(second_child_page.next_cursor.is_none());

    let grandchildren = service
      .list_session_children(ListSessionChildrenRequest {
        parent_session_key: child.session_key.clone(),
        cursor: None,
        offset: None,
        limit: None,
      })
      .unwrap();
    assert_eq!(grandchildren.sessions.len(), 1);
    assert_eq!(grandchildren.sessions[0].session_id, "grandchild");
    assert_eq!(grandchildren.sessions[0].child_count, 0);
  }

  #[test]
  fn relation_index_promotes_orphans_and_breaks_cycles_without_losing_sessions() {
    let service = ViewerService::new(Arc::new(FakeRepository {
      listings: HashMap::from([(
        ViewerProvider::Codex,
        Ok(vec![
          session_header("orphan", Some("missing"), "/projects/Alpha", "4000"),
          session_header("cycle-a", Some("cycle-b"), "/projects/Alpha", "3000"),
          session_header("cycle-b", Some("cycle-a"), "/projects/Alpha", "2000"),
        ]),
      )]),
      loaded: Mutex::new(None),
    }));

    let roots = service
      .list_sessions(ListSessionsRequest {
        query: SessionQuery::default(),
        cursor: None,
        offset: None,
        limit: None,
      })
      .unwrap();
    let root_ids = roots
      .sessions
      .iter()
      .map(|session| session.session_id.as_str())
      .collect::<Vec<_>>();
    assert!(root_ids.contains(&"orphan"));
    assert!(root_ids.contains(&"cycle-b"));
    assert!(roots.sessions.iter().all(|session| !session.is_subagent));

    let cycle_root = roots
      .sessions
      .iter()
      .find(|session| session.session_id == "cycle-b")
      .expect("a cycle node becomes a root");
    let children = service
      .list_session_children(ListSessionChildrenRequest {
        parent_session_key: cycle_root.session_key.clone(),
        cursor: None,
        offset: None,
        limit: None,
      })
      .unwrap();
    assert_eq!(children.sessions.len(), 1);
    assert_eq!(children.sessions[0].session_id, "cycle-a");

    let descendant_children = service
      .list_session_children(ListSessionChildrenRequest {
        parent_session_key: children.sessions[0].session_key.clone(),
        cursor: None,
        offset: None,
        limit: None,
      })
      .unwrap();
    assert!(descendant_children.sessions.is_empty());
  }

  #[cfg(unix)]
  #[test]
  fn listing_isolates_a_non_utf8_source_path_to_its_provider_warning() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let mut invalid = session_header("invalid", None, "/projects/Alpha", "2000");
    invalid.path = PathBuf::from(OsString::from_vec(b"/fixtures/invalid-\xff.jsonl".to_vec()));
    let valid = session_header("valid", None, "/projects/Alpha", "1000");
    let service = ViewerService::new(Arc::new(FakeRepository {
      listings: HashMap::from([(ViewerProvider::Codex, Ok(vec![invalid, valid]))]),
      loaded: Mutex::new(None),
    }));

    let response = service
      .list_sessions(ListSessionsRequest {
        query: SessionQuery::default(),
        cursor: None,
        offset: None,
        limit: None,
      })
      .unwrap();

    assert_eq!(response.sessions.len(), 1);
    assert_eq!(response.sessions[0].session_id, "valid");
    assert_eq!(response.source_errors.len(), 1);
    assert_eq!(response.source_errors[0].provider, ViewerProvider::Codex);
  }

  #[test]
  fn session_metadata_and_agent_identity_are_sanitized_and_bounded() {
    let mut header = session_header("session-id", None, "/projects/Alpha", "1000");
    header.agent_nickname = Some("worker\u{202e}-name".to_string());
    header.agent_role = Some(format!("role {}", "x".repeat(MAX_AGENT_IDENTITY_CHARS + 20)));
    header.title = Some(" \u{001b}[31mBuild\u{202e}\n\tthe viewer\u{001b}[0m ".to_string());
    header.preview = Some(format!("Prompt {}", "x".repeat(MAX_SESSION_PREVIEW_CHARS + 20)));

    let summary = session_summary(ViewerProvider::Pi, header).unwrap();

    assert_eq!(summary.title.as_deref(), Some("Build the viewer"));
    assert_eq!(
      summary.preview.as_ref().unwrap().chars().count(),
      MAX_SESSION_PREVIEW_CHARS
    );
    assert!(summary.preview.as_ref().unwrap().ends_with('\u{2026}'));
    assert_eq!(summary.agent_nickname.as_deref(), Some("worker-name"));
    assert_eq!(
      summary.agent_role.as_ref().unwrap().chars().count(),
      MAX_AGENT_IDENTITY_CHARS
    );
    assert!(summary.agent_role.as_ref().unwrap().ends_with('\u{2026}'));
  }

  #[test]
  fn search_matches_native_titles_and_first_message_previews() {
    let mut title_header = session_header("title", None, "/projects/Alpha", "2000");
    title_header.title = Some("Release checklist".to_string());
    let mut preview_header = session_header("preview", None, "/projects/Alpha", "1000");
    preview_header.preview = Some("Investigate socket ownership".to_string());
    let service = ViewerService::new(Arc::new(FakeRepository {
      listings: HashMap::from([(ViewerProvider::Codex, Ok(vec![title_header, preview_header]))]),
      loaded: Mutex::new(None),
    }));

    let response = service
      .list_sessions(ListSessionsRequest {
        query: SessionQuery {
          providers: vec![ViewerProvider::Codex],
          search: Some("socket ownership".to_string()),
        },
        cursor: None,
        offset: None,
        limit: None,
      })
      .unwrap();

    assert_eq!(response.sessions.len(), 1);
    assert_eq!(response.sessions[0].session_id, "preview");
  }

  #[test]
  fn default_listing_hydrates_only_the_visible_page_and_caches_by_source_revision() {
    let directory = tempfile::tempdir().unwrap();
    let mut headers = vec![
      session_header("newest", None, "/projects/Alpha", "3000"),
      session_header("middle", None, "/projects/Alpha", "2000"),
      session_header("oldest", None, "/projects/Alpha", "1000"),
    ];
    for header in &mut headers {
      header.path = directory.path().join(format!("{}.jsonl", header.id));
      std::fs::write(&header.path, "fixture\n").unwrap();
    }
    let hydrations = Arc::new(AtomicUsize::new(0));
    let service = ViewerService::new(Arc::new(HydratingRepository {
      headers,
      hydrations: Arc::clone(&hydrations),
    }));
    let request = ListSessionsRequest {
      query: SessionQuery {
        providers: vec![ViewerProvider::Codex],
        search: None,
      },
      cursor: None,
      offset: None,
      limit: Some(1),
    };

    let first = service.list_sessions(request.clone()).unwrap();
    let second = service.list_sessions(request).unwrap();

    assert_eq!(first.sessions[0].session_id, "newest");
    assert_eq!(first.sessions[0].preview.as_deref(), Some("Prompt for newest"));
    assert_eq!(second.sessions[0].preview, first.sessions[0].preview);
    assert_eq!(hydrations.load(Ordering::SeqCst), 1);
  }

  #[test]
  fn cheap_field_search_hydrates_only_the_visible_matching_page() {
    let headers = vec![
      session_header("newest", None, "/projects/Alpha", "3000"),
      session_header("middle", None, "/projects/Alpha", "2000"),
      session_header("oldest", None, "/projects/Alpha", "1000"),
    ];
    let hydrations = Arc::new(AtomicUsize::new(0));
    let service = ViewerService::new(Arc::new(HydratingRepository {
      headers,
      hydrations: Arc::clone(&hydrations),
    }));

    let response = service
      .list_sessions(ListSessionsRequest {
        query: SessionQuery {
          providers: vec![ViewerProvider::Codex],
          search: Some("alpha".to_string()),
        },
        cursor: None,
        offset: None,
        limit: Some(1),
      })
      .unwrap();

    assert_eq!(response.sessions[0].session_id, "newest");
    assert_eq!(response.sessions[0].preview.as_deref(), Some("Prompt for newest"));
    assert_eq!(hydrations.load(Ordering::SeqCst), 1);
    assert!(response.next_cursor.is_some());
  }

  #[test]
  fn transient_hydration_failures_are_not_cached() {
    let directory = tempfile::tempdir().unwrap();
    let mut header = session_header("flaky", None, "/projects/Alpha", "1000");
    header.path = directory.path().join("flaky.jsonl");
    std::fs::write(&header.path, "fixture\n").unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let service = ViewerService::new(Arc::new(FlakyHydratingRepository {
      header,
      attempts: Arc::clone(&attempts),
    }));
    let request = ListSessionsRequest {
      query: SessionQuery {
        providers: vec![ViewerProvider::Pi],
        search: None,
      },
      cursor: None,
      offset: None,
      limit: Some(1),
    };

    let first = service.list_sessions(request.clone()).unwrap();
    let second = service.list_sessions(request).unwrap();

    assert_eq!(first.sessions[0].preview, None);
    assert_eq!(second.sessions[0].preview.as_deref(), Some("Recovered prompt"));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
  }

  #[test]
  fn overlapping_requests_share_one_in_flight_session_hydration() {
    let directory = tempfile::tempdir().unwrap();
    let mut header = session_header("shared", None, "/projects/Alpha", "1000");
    header.path = directory.path().join("shared.jsonl");
    std::fs::write(&header.path, "fixture\n").unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let service = ViewerService::new(Arc::new(SlowHydratingRepository {
      header,
      attempts: Arc::clone(&attempts),
    }));
    let request = ListSessionsRequest {
      query: SessionQuery {
        providers: vec![ViewerProvider::Dsh],
        search: None,
      },
      cursor: None,
      offset: None,
      limit: Some(1),
    };
    let start = Arc::new(Barrier::new(3));
    let handles: Vec<_> = (0..2)
      .map(|_| {
        let service = service.clone();
        let request = request.clone();
        let start = Arc::clone(&start);
        std::thread::spawn(move || {
          start.wait();
          service.list_sessions(request).unwrap()
        })
      })
      .collect();
    start.wait();

    for handle in handles {
      let response = handle.join().unwrap();
      assert_eq!(response.sessions[0].preview.as_deref(), Some("Shared prompt"));
    }
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
  }

  #[test]
  fn event_pages_are_bounded_and_keep_absolute_event_keys() {
    let service = service_with_session(visible_session(5));
    let session_key = key_for("fixture");

    let page = service
      .load_event_page(EventPageRequest {
        session_key,
        cursor: None,
        offset: Some(1),
        direction: PageDirection::Forward,
        limit: Some(2),
      })
      .unwrap();

    assert_eq!(page.events.len(), 2);
    assert_eq!(decode_event_key(&page.events[0].event_key).unwrap(), 1);
    assert_eq!(decode_event_key(&page.events[1].event_key).unwrap(), 2);
    assert_eq!(page.events[0].role.as_deref(), Some("assistant"));
    assert_eq!(page.events[0].summary, "message 1");
    assert!(!page.events[0].summary_truncated);
    assert_eq!(decode_event_cursor(page.next_cursor.as_deref().unwrap()).unwrap(), 3);
    assert_eq!(
      decode_event_cursor(page.previous_cursor.as_deref().unwrap()).unwrap(),
      1
    );
    assert_eq!(page.total_events, 5);
  }

  #[test]
  fn listing_orders_providers_by_explicit_update_time_not_creation_time() {
    let listings = HashMap::from([
      (
        ViewerProvider::Codex,
        Ok(vec![session_header_with_updated(
          "created-first",
          "/projects/one",
          "2026-08-31T10:00:00Z",
          "100",
          100,
        )]),
      ),
      (
        ViewerProvider::Pi,
        Ok(vec![session_header_with_updated(
          "updated-first",
          "/projects/two",
          "2026-08-30T10:00:00Z",
          "200",
          200,
        )]),
      ),
    ]);
    let service = ViewerService::new(Arc::new(FakeRepository {
      listings,
      loaded: Mutex::new(None),
    }));

    let response = service
      .list_sessions(ListSessionsRequest {
        query: SessionQuery::default(),
        cursor: None,
        offset: None,
        limit: None,
      })
      .unwrap();

    assert_eq!(response.sessions[0].session_id, "updated-first");
    assert_eq!(response.sessions[0].updated_at_ms, Some(200));
    assert_eq!(response.sessions[1].session_id, "created-first");
  }

  #[test]
  fn backward_event_pages_return_chronological_slices() {
    let service = service_with_session(visible_session(5));
    let page = service
      .load_event_page(EventPageRequest {
        session_key: key_for("fixture"),
        cursor: None,
        offset: None,
        direction: PageDirection::Backward,
        limit: Some(2),
      })
      .unwrap();

    assert_eq!(decode_event_key(&page.events[0].event_key).unwrap(), 3);
    assert_eq!(decode_event_key(&page.events[1].event_key).unwrap(), 4);
    assert!(page.next_cursor.is_none());
    assert_eq!(
      decode_event_cursor(page.previous_cursor.as_deref().unwrap()).unwrap(),
      3
    );
  }

  #[test]
  fn event_pages_use_normalized_ir_discriminants() {
    let tool = AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id: Some("fixture".to_string()),
      turn_id: None,
      message_id: None,
      parent_id: None,
      record_kind: ToolRecordKind::Snapshot,
      tool_call_id: Some("call-1".to_string()),
      provider_tool_name: Some("shell".to_string()),
      tool_name: Some("shell".to_string()),
      tool_kind: ToolKind::Shell,
      transport: Some(ToolTransport::Native),
      summary: None,
      phase: Phase::Finished,
      input: None,
      output: None,
      is_error: Some(false),
      native: None,
      timestamp: None,
    });
    let page = service_with_session(loaded_session(vec![tool]))
      .load_event_page(EventPageRequest {
        session_key: key_for("fixture"),
        cursor: None,
        offset: None,
        direction: PageDirection::Forward,
        limit: None,
      })
      .unwrap();

    assert_eq!(page.events[0].event_type, "tool_call");
  }

  #[test]
  fn tool_cards_project_every_known_summary_and_keep_an_unknown_fallback() {
    let events = vec![
      tool_call(
        Provider::Codex,
        "shell",
        "shell-1",
        ToolKind::Shell,
        Some(ToolSummary::Shell {
          command: Some("cargo test".to_string()),
          cwd: Some("/work".to_string()),
          exit_code: Some(0),
        }),
        Phase::Finished,
        None,
      ),
      tool_call(
        Provider::Codex,
        "read_file",
        "read-1",
        ToolKind::FileRead,
        Some(ToolSummary::FileRead {
          path: Some("src/lib.rs".to_string()),
        }),
        Phase::Finished,
        None,
      ),
      tool_call(
        Provider::Codex,
        "write_file",
        "write-1",
        ToolKind::FileWrite,
        Some(ToolSummary::FileWrite {
          path: Some("out.txt".to_string()),
          bytes: Some(42),
        }),
        Phase::Finished,
        None,
      ),
      tool_call(
        Provider::Codex,
        "apply_patch",
        "edit-1",
        ToolKind::FileEdit,
        Some(ToolSummary::FileEdit {
          path: Some("src/main.rs".to_string()),
          added: Some(4),
          removed: Some(2),
        }),
        Phase::Finished,
        None,
      ),
      tool_call(
        Provider::Codex,
        "search",
        "search-1",
        ToolKind::Search,
        Some(ToolSummary::Search {
          query: Some("ToolCallEvent".to_string()),
        }),
        Phase::Finished,
        None,
      ),
      tool_call(
        Provider::Codex,
        "fetch",
        "web-1",
        ToolKind::Web,
        Some(ToolSummary::Web {
          url: Some("https://example.test".to_string()),
        }),
        Phase::Finished,
        None,
      ),
      tool_call(
        Provider::Codex,
        "task",
        "task-1",
        ToolKind::Task,
        Some(ToolSummary::Task {
          title: Some("Run checks".to_string()),
        }),
        Phase::Finished,
        None,
      ),
      tool_call(
        Provider::Codex,
        "future_tool",
        "unknown-1",
        ToolKind::Unknown,
        None,
        Phase::Finished,
        None,
      ),
    ];

    let summaries: Vec<EventSummary> = events
      .iter()
      .enumerate()
      .map(|(index, event)| event_summary(&events, index, event))
      .collect();

    let shell = summaries[0].tool.as_ref().unwrap();
    assert_eq!(shell.kind, "shell");
    assert_eq!(shell.command.as_deref(), Some("cargo test"));
    assert_eq!(shell.cwd.as_deref(), Some("/work"));
    assert_eq!(shell.exit_code, Some(0));
    assert_eq!(summaries[1].tool.as_ref().unwrap().path.as_deref(), Some("src/lib.rs"));
    assert_eq!(summaries[2].tool.as_ref().unwrap().bytes, Some(42));
    assert_eq!(summaries[3].tool.as_ref().unwrap().added, Some(4));
    assert_eq!(summaries[3].tool.as_ref().unwrap().removed, Some(2));
    assert_eq!(
      summaries[4].tool.as_ref().unwrap().query.as_deref(),
      Some("ToolCallEvent")
    );
    assert_eq!(
      summaries[5].tool.as_ref().unwrap().url.as_deref(),
      Some("https://example.test")
    );
    assert_eq!(
      summaries[6].tool.as_ref().unwrap().task_title.as_deref(),
      Some("Run checks")
    );
    let unknown = summaries[7].tool.as_ref().unwrap();
    assert_eq!(unknown.kind, "unknown");
    assert_eq!(unknown.tool_name.as_deref(), Some("future_tool"));
    assert_eq!(unknown.tool_call_id.as_deref(), Some("unknown-1"));
  }

  #[test]
  fn tool_operation_cards_are_bounded_and_replace_lifecycle_fragments() {
    let long_command = "c".repeat(MAX_TOOL_CARD_STRING_CHARS + 100);
    let events = vec![
      tool_call(
        Provider::Codex,
        "exec_command",
        "call-1",
        ToolKind::Shell,
        Some(ToolSummary::Shell {
          command: Some(long_command),
          cwd: Some("/work".to_string()),
          exit_code: None,
        }),
        Phase::Started,
        None,
      ),
      AgentEvent::Message(MessageEvent {
        provenance: None,
        provider: Provider::Codex,
        session_id: Some("fixture".to_string()),
        message_id: Some("between".to_string()),
        parent_id: None,
        role: Role::Assistant,
        delivery: MessageDelivery::Commentary,
        phase: Phase::Finished,
        text: "working".to_string(),
        timestamp: None,
      }),
      tool_call(
        Provider::Codex,
        "exec_command",
        "call-1",
        ToolKind::Shell,
        Some(ToolSummary::Shell {
          command: None,
          cwd: None,
          exit_code: Some(7),
        }),
        Phase::Finished,
        Some(json!({"stdout": "failed"})),
      ),
    ];

    let page = service_with_session(loaded_session(events))
      .load_event_page(EventPageRequest {
        session_key: key_for("fixture"),
        cursor: None,
        offset: None,
        direction: PageDirection::Forward,
        limit: None,
      })
      .unwrap();
    assert_eq!(page.total_events, 2);
    assert_eq!(page.events.len(), 2);
    assert_eq!(page.events[0].event_type, "message");
    let operation = &page.events[1];
    let operation_tool = operation.tool.as_ref().unwrap();

    assert_eq!(
      operation_tool.command.as_ref().unwrap().chars().count(),
      MAX_TOOL_CARD_STRING_CHARS
    );
    assert!(operation_tool.command.as_ref().unwrap().ends_with('\u{2026}'));
    assert_eq!(operation_tool.exit_code, Some(7));
    assert_eq!(operation_tool.status, "failed");
    assert_eq!(operation.is_error, Some(true));
    assert_eq!(operation_tool.cwd.as_deref(), Some("/work"));
    assert_eq!(decode_event_key(&operation.event_key).unwrap(), 0);
  }

  #[test]
  fn tool_operation_detail_exposes_final_output_without_a_private_viewer_join() {
    let mut invocation = tool_call(
      Provider::Codex,
      "exec_command",
      "call-1",
      ToolKind::Shell,
      None,
      Phase::Finished,
      None,
    );
    let AgentEvent::ToolCall(invocation_event) = &mut invocation else {
      unreachable!();
    };
    invocation_event.record_kind = ToolRecordKind::Invocation;
    invocation_event.input = Some(json!({"cmd": "cargo test"}));
    let events = vec![
      invocation,
      tool_call(
        Provider::Codex,
        "exec_command",
        "call-1",
        ToolKind::Shell,
        None,
        Phase::Delta,
        Some(Value::String("partial".to_string())),
      ),
      tool_call(
        Provider::Codex,
        "exec_command",
        "call-1",
        ToolKind::Shell,
        None,
        Phase::Finished,
        Some(json!({"output": {"content": [{"type": "text", "text": "final"}]}})),
      ),
    ];
    let detail = service_with_session(loaded_session(events))
      .load_event_detail(LoadEventDetailRequest {
        session_key: key_for("fixture"),
        event_key: encode_event_key(0),
      })
      .unwrap();
    let output = detail.tool_output.unwrap();

    assert_eq!(output.source_event_key, encode_event_key(0));
    assert!(!output.truncated);
    assert_eq!(output.sections.len(), 1);
    assert_eq!(output.sections[0].text, "final");
    assert_eq!(output.sections[0].format, "text");
    assert_eq!(detail.event["source_event_indices"], json!([0, 1, 2]));
  }

  #[test]
  fn terminal_operation_keeps_semantic_output_and_both_code_mode_records() {
    let mut invocation = tool_call(
      Provider::Codex,
      "write_stdin",
      "call-write",
      ToolKind::Terminal,
      Some(ToolSummary::Terminal {
        session_id: Some("90855".to_string()),
        action: Some(TerminalAction::Wait),
        chars_len: Some(0),
        wait_ms: Some(30_000),
      }),
      Phase::Started,
      None,
    );
    let AgentEvent::ToolCall(invocation_call) = &mut invocation else {
      unreachable!();
    };
    invocation_call.provider_tool_name = Some("exec".to_string());
    invocation_call.transport = Some(ToolTransport::CodeExecution);
    invocation_call.input = Some(json!({
      "session_id": 90855,
      "chars": "",
      "yield_time_ms": 30_000,
    }));
    invocation_call.native = Some(json!({
      "type": "custom_tool_call",
      "input": "const r = await tools.write_stdin(...);",
    }));

    let mut result = tool_call(
      Provider::Codex,
      "write_stdin",
      "call-write",
      ToolKind::Terminal,
      None,
      Phase::Finished,
      Some(json!({
        "session_id": 90855,
        "wall_time_seconds": 30.001,
        "text": "Refreshing checks status",
      })),
    );
    let AgentEvent::ToolCall(result_call) = &mut result else {
      unreachable!();
    };
    result_call.provider_tool_name = Some("exec".to_string());
    result_call.transport = Some(ToolTransport::CodeExecution);
    result_call.native = Some(json!({
      "type": "custom_tool_call_output",
      "output": ["Script completed", "{...}"],
    }));

    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("code-mode.jsonl");
    std::fs::write(&source_path, "fixture\n").unwrap();
    let session_key = encode_session_key(&SessionLocator {
      version: 1,
      provider: ViewerProvider::Codex,
      session_id: "fixture".to_string(),
      source_path,
    })
    .unwrap();
    let service = service_with_session(loaded_session(vec![invocation, result]));
    let page = service
      .load_event_page(EventPageRequest {
        session_key: session_key.clone(),
        cursor: None,
        offset: None,
        direction: PageDirection::Forward,
        limit: None,
      })
      .unwrap();

    assert_eq!(page.total_events, 1);
    let card = page.events[0].tool.as_ref().unwrap();
    assert_eq!(card.kind, "terminal");
    assert_eq!(card.status, "completed");
    assert_eq!(card.provider_tool_name.as_deref(), Some("exec"));
    assert_eq!(card.terminal_session_id.as_deref(), Some("90855"));
    assert_eq!(card.terminal_action.as_deref(), Some("wait"));
    assert_eq!(card.wait_ms, Some(30_000));

    let detail = service
      .load_event_detail(LoadEventDetailRequest {
        session_key,
        event_key: page.events[0].event_key.clone(),
      })
      .unwrap();
    assert_eq!(detail.event["output"]["text"], "Refreshing checks status");
    assert!(detail.event.get("native").is_none());
    assert_eq!(
      detail.tool_output.as_ref().unwrap().sections[0].text,
      "Refreshing checks status"
    );
    assert_eq!(
      detail.native.as_ref().unwrap()["source_records"]
        .as_array()
        .unwrap()
        .len(),
      2
    );
    assert_eq!(
      detail.native.as_ref().unwrap()["source_records"][0]["native"]["type"],
      "custom_tool_call"
    );
    assert_eq!(
      detail.native.as_ref().unwrap()["source_records"][1]["native"]["type"],
      "custom_tool_call_output"
    );
  }

  #[test]
  fn tool_operation_assembly_keeps_missing_and_ambiguous_ids_separate() {
    let missing_ids = vec![
      tool_call(
        Provider::Codex,
        "exec_command",
        "",
        ToolKind::Shell,
        None,
        Phase::Started,
        None,
      ),
      tool_call(
        Provider::Codex,
        "exec_command",
        "",
        ToolKind::Shell,
        None,
        Phase::Finished,
        Some(Value::String("wrong".to_string())),
      ),
    ];
    assert_eq!(tokn_session_core::assemble_tool_operations(&missing_ids).len(), 2);

    let mut first = with_tool_input(
      tool_call(
        Provider::Codex,
        "exec_command",
        "reused",
        ToolKind::Shell,
        None,
        Phase::Started,
        None,
      ),
      json!({"cmd": "first"}),
    );
    let mut second = with_tool_input(
      tool_call(
        Provider::Codex,
        "exec_command",
        "reused",
        ToolKind::Shell,
        None,
        Phase::Started,
        None,
      ),
      json!({"cmd": "second"}),
    );
    let AgentEvent::ToolCall(first_tool) = &mut first else {
      unreachable!();
    };
    first_tool.record_kind = ToolRecordKind::Invocation;
    let AgentEvent::ToolCall(second_tool) = &mut second else {
      unreachable!();
    };
    second_tool.record_kind = ToolRecordKind::Invocation;
    let events = vec![
      first,
      second,
      tool_call(
        Provider::Codex,
        "exec_command",
        "reused",
        ToolKind::Shell,
        None,
        Phase::Finished,
        Some(Value::String("ambiguous result".to_string())),
      ),
    ];
    let entries = timeline_entries(&events);
    assert_eq!(entries.len(), 3);
    assert!(matches!(entries[0], TimelineEntry::ToolOperation { .. }));
    assert!(matches!(entries[1], TimelineEntry::ToolOperation { .. }));
    assert!(matches!(entries[2], TimelineEntry::ToolOperation { .. }));
  }

  #[test]
  fn output_projection_handles_shell_opencode_dsh_and_json_fallbacks() {
    let shell = project_output(
      &json!({
        "formatted_output": "formatted",
        "aggregated_output": "aggregated",
        "stdout": "stdout",
        "stderr": "stderr"
      }),
      0,
    );
    assert_eq!(shell.len(), 2);
    assert_eq!(shell[0].label.as_deref(), Some("Output"));
    assert_eq!(shell[0].text, "formatted");
    assert_eq!(shell[1].label.as_deref(), Some("Stderr"));
    assert_eq!(shell[1].text, "stderr");

    let opencode = project_output(&json!({"output": {"result": "nested"}, "metadata": {"exit": 0}}), 0);
    assert_eq!(opencode[0].label.as_deref(), Some("Result"));
    assert_eq!(opencode[0].text, "nested");
    assert_eq!(opencode[0].format, "text");

    let dsh = project_output(
      &json!({
        "content": [{
          "type": "tool-result",
          "content": [
            {"type": "text", "text": "first"},
            {"type": "text", "text": "second"}
          ]
        }],
        "error": null
      }),
      0,
    );
    assert_eq!(dsh[0].text, "first\nsecond");
    assert_eq!(dsh[0].format, "text");

    let dynamic_error = project_output(
      &json!({
        "content_items": [{"type": "text", "text": "partial result"}],
        "success": false,
        "error": "tool failed"
      }),
      0,
    );
    assert_eq!(dynamic_error.len(), 2);
    assert_eq!(dynamic_error[0].text, "partial result");
    assert_eq!(dynamic_error[1].label.as_deref(), Some("Error"));
    assert_eq!(dynamic_error[1].text, "tool failed");

    let fallback = project_output(&json!({"future": {"value": 1}}), 0);
    assert_eq!(fallback.len(), 1);
    assert_eq!(fallback[0].format, "json");
    assert!(fallback[0].text.contains("\"future\""));
  }

  #[test]
  fn tool_output_preview_is_utf8_safe_and_keeps_both_ends() {
    let text = format!("HEAD{}TAIL", "\u{1f642}".repeat(MAX_TOOL_OUTPUT_BYTES / 2));
    let event = tool_call(
      Provider::Codex,
      "exec_command",
      "call-1",
      ToolKind::Shell,
      None,
      Phase::Finished,
      Some(Value::String(text.clone())),
    );
    let output = tool_output_preview(std::slice::from_ref(&event), 0).unwrap();

    assert!(output.truncated);
    assert_eq!(output.original_size_bytes, text.len());
    assert!(output.sections[0].text.len() <= MAX_TOOL_OUTPUT_BYTES);
    assert!(output.sections[0].text.starts_with("HEAD"));
    assert!(output.sections[0].text.ends_with("TAIL"));
    assert!(output.sections[0].text.contains("output truncated"));
  }

  #[test]
  fn message_summaries_preserve_markdown_within_the_timeline_cap() {
    let markdown = "# Result\n\n```rust\nfn main() {}\n```\n";
    let page = service_with_session(loaded_session(vec![message_event(markdown)]))
      .load_event_page(EventPageRequest {
        session_key: key_for("fixture"),
        cursor: None,
        offset: None,
        direction: PageDirection::Forward,
        limit: None,
      })
      .unwrap();

    assert_eq!(page.events[0].summary, markdown);
    assert!(!page.events[0].summary_truncated);
  }

  #[test]
  fn reasoning_summaries_keep_only_the_safe_card_preview() {
    let markdown = "## Approach\n\n- inspect the source\n- verify the result\n";
    let event = AgentEvent::Reasoning(ReasoningEvent {
      provenance: None,
      provider: Provider::Codex,
      session_id: Some("fixture".to_string()),
      message_id: Some("reasoning".to_string()),
      parent_id: None,
      phase: Phase::Finished,
      text: Some("Longer private reasoning".to_string()),
      summary: Some(markdown.to_string()),
      redacted: None,
      encrypted_content: None,
      signature: None,
      timestamp: None,
    });
    let summary = event_summary(std::slice::from_ref(&event), 0, &event);

    assert_eq!(summary.summary, "## Approach - inspect the source - verify the result");
    assert!(!summary.summary_truncated);
    assert_eq!(
      summary.reasoning.as_ref().and_then(|card| card.preview.as_deref()),
      Some(summary.summary.as_str())
    );
    assert!(
      !serde_json::to_string(&summary)
        .unwrap()
        .contains("Longer private reasoning")
    );
  }

  #[test]
  fn usage_cards_preserve_scopes_optional_values_and_u64_precision() {
    let mut model_call = usage_event(UsageKind::ModelCall, Provider::Codex);
    let AgentEvent::Usage(model_call_usage) = &mut model_call else {
      panic!("fixture must be a usage event");
    };
    model_call_usage.input_tokens = u64::MAX;
    model_call_usage.output_tokens = 0;
    model_call_usage.total_tokens = None;
    model_call_usage.cache_read_tokens = Some(0);
    model_call_usage.cache_write_tokens = None;
    model_call_usage.reasoning_tokens = Some(u64::MAX);
    model_call_usage.turn_id = Some("turn-1".to_string());
    model_call_usage.step_id = Some("step-1".to_string());

    let mut operation_total = usage_event(UsageKind::OperationTotal, Provider::OpenCode);
    let AgentEvent::Usage(operation_total_usage) = &mut operation_total else {
      panic!("fixture must be a usage event");
    };
    operation_total_usage.input_tokens = 7;
    operation_total_usage.output_tokens = 11;
    operation_total_usage.total_tokens = Some(19);
    operation_total_usage.cache_read_tokens = Some(3);
    operation_total_usage.cache_write_tokens = Some(5);
    operation_total_usage.reasoning_tokens = Some(0);
    operation_total_usage.turn_id = Some("  ".to_string());
    operation_total_usage.step_id = None;

    let mut session_snapshot = usage_event(UsageKind::SessionSnapshot, Provider::Dsh);
    let AgentEvent::Usage(session_snapshot_usage) = &mut session_snapshot else {
      panic!("fixture must be a usage event");
    };
    session_snapshot_usage.input_tokens = 0;
    session_snapshot_usage.output_tokens = 0;
    session_snapshot_usage.total_tokens = Some(0);

    let events = vec![model_call, operation_total, session_snapshot];
    let summaries: Vec<EventSummary> = events
      .iter()
      .enumerate()
      .map(|(index, event)| event_summary(&events, index, event))
      .collect();

    let model_call = summaries[0].usage.as_ref().expect("model-call card");
    assert_eq!(model_call.kind, "model_call");
    assert_eq!(model_call.input_tokens, u64::MAX.to_string());
    assert_eq!(model_call.output_tokens, "0");
    assert_eq!(model_call.total_tokens, None);
    assert_eq!(model_call.cache_read_tokens.as_deref(), Some("0"));
    assert_eq!(model_call.cache_write_tokens, None);
    assert_eq!(model_call.reasoning_tokens, Some(u64::MAX.to_string()));
    assert_eq!(model_call.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(model_call.step_id.as_deref(), Some("step-1"));

    let operation_total = summaries[1].usage.as_ref().expect("operation-total card");
    assert_eq!(operation_total.kind, "operation_total");
    assert_eq!(operation_total.total_tokens.as_deref(), Some("19"));
    assert_eq!(operation_total.cache_read_tokens.as_deref(), Some("3"));
    assert_eq!(operation_total.cache_write_tokens.as_deref(), Some("5"));
    assert_eq!(operation_total.reasoning_tokens.as_deref(), Some("0"));
    assert_eq!(operation_total.turn_id, None);
    assert_eq!(operation_total.step_id, None);

    let session_snapshot = summaries[2].usage.as_ref().expect("session-snapshot card");
    assert_eq!(session_snapshot.kind, "session_snapshot");
    assert_eq!(session_snapshot.input_tokens, "0");
    assert_eq!(session_snapshot.output_tokens, "0");
    assert_eq!(session_snapshot.total_tokens.as_deref(), Some("0"));

    let serialized = serde_json::to_value(&summaries[0]).unwrap();
    assert!(serialized["usage"]["input_tokens"].is_string());
    assert_eq!(serialized["usage"]["input_tokens"], u64::MAX.to_string());
    assert!(serialized["usage"]["total_tokens"].is_null());
    assert_eq!(serialized["usage"]["cache_read_tokens"], "0");
  }

  #[test]
  fn reasoning_cards_classify_safe_content_and_redaction() {
    let events = vec![
      reasoning_event(
        Some("Summary\nwith \u{001b}[31mcolor\u{001b}[0m"),
        Some("Detailed reasoning"),
        Some("encrypted-secret"),
        Some("signature-secret"),
        None,
      ),
      reasoning_event(None, Some("Text-only reasoning"), None, None, None),
      reasoning_event(None, None, Some("ciphertext-secret"), Some("signature-secret"), None),
      reasoning_event(
        Some("redacted-summary-secret"),
        Some("redacted-text-secret"),
        Some("redacted-ciphertext-secret"),
        Some("redacted-signature-secret"),
        Some(true),
      ),
    ];
    let summaries: Vec<EventSummary> = events
      .iter()
      .enumerate()
      .map(|(index, event)| event_summary(&events, index, event))
      .collect();

    let rich = summaries[0].reasoning.as_ref().expect("reasoning card");
    assert_eq!(rich.preview.as_deref(), Some("Summary with color"));
    assert!(rich.has_summary);
    assert!(rich.has_text);
    assert!(rich.has_encrypted_content);
    assert!(!rich.is_redacted);
    let rich_json = serde_json::to_string(&summaries[0]).unwrap();
    assert!(!rich_json.contains("encrypted-secret"));
    assert!(!rich_json.contains("signature-secret"));

    let text_only = summaries[1].reasoning.as_ref().expect("text-only reasoning card");
    assert_eq!(text_only.preview.as_deref(), Some("Text-only reasoning"));
    assert!(!text_only.has_summary);
    assert!(text_only.has_text);
    assert!(!text_only.has_encrypted_content);
    assert!(!text_only.is_redacted);

    let encrypted = summaries[2].reasoning.as_ref().expect("encrypted reasoning card");
    assert_eq!(encrypted.preview, None);
    assert!(!encrypted.has_summary);
    assert!(!encrypted.has_text);
    assert!(encrypted.has_encrypted_content);
    assert!(!encrypted.is_redacted);
    let encrypted_json = serde_json::to_string(&summaries[2]).unwrap();
    assert!(!encrypted_json.contains("ciphertext-secret"));
    assert!(!encrypted_json.contains("signature-secret"));

    let redacted = summaries[3].reasoning.as_ref().expect("redacted reasoning card");
    assert_eq!(redacted.preview, None);
    assert!(redacted.has_summary);
    assert!(redacted.has_text);
    assert!(redacted.has_encrypted_content);
    assert!(redacted.is_redacted);
    let redacted_json = serde_json::to_string(&summaries[3]).unwrap();
    assert!(!redacted_json.contains("redacted-summary-secret"));
    assert!(!redacted_json.contains("redacted-text-secret"));
    assert!(!redacted_json.contains("redacted-ciphertext-secret"));
    assert!(!redacted_json.contains("redacted-signature-secret"));
  }

  #[test]
  fn reasoning_card_previews_are_single_line_and_bounded() {
    let event = reasoning_event(
      Some(&format!(
        "\u{001b}[31mFirst\nsecond\u{202e} {}",
        "x".repeat(MAX_REASONING_CARD_PREVIEW_CHARS)
      )),
      None,
      None,
      None,
      None,
    );
    let summary = event_summary(std::slice::from_ref(&event), 0, &event);
    let preview = summary
      .reasoning
      .as_ref()
      .and_then(|reasoning| reasoning.preview.as_deref())
      .expect("safe preview");

    assert_eq!(preview.chars().count(), MAX_REASONING_CARD_PREVIEW_CHARS);
    assert!(preview.starts_with("First second "));
    assert!(preview.ends_with('\u{2026}'));
    assert!(!preview.contains('\n'));
    assert!(!preview.contains('\u{001b}'));
    assert!(!preview.contains('\u{202e}'));
  }

  #[test]
  fn truncated_message_summaries_keep_full_bounded_inspector_content() {
    let full_text = "m".repeat(MAX_MESSAGE_SUMMARY_CHARS + 1);
    let page = service_with_session(loaded_session(vec![message_event(&full_text)]))
      .load_event_page(EventPageRequest {
        session_key: key_for("fixture"),
        cursor: None,
        offset: None,
        direction: PageDirection::Forward,
        limit: None,
      })
      .unwrap();

    assert!(page.events[0].summary_truncated);
    assert_eq!(page.events[0].summary.chars().count(), MAX_MESSAGE_SUMMARY_CHARS);
    assert!(page.events[0].summary.ends_with('…'));

    let detail = service_with_session(loaded_session(vec![message_event(&full_text)]))
      .load_event_detail(LoadEventDetailRequest {
        session_key: key_for("fixture"),
        event_key: encode_event_key(0),
      })
      .unwrap();
    assert_eq!(detail.event["text"], full_text);
  }

  #[test]
  fn technical_summaries_keep_the_compact_timeline_cap() {
    let event = AgentEvent::Error(ErrorEvent {
      provider: Provider::Codex,
      session_id: Some("fixture".to_string()),
      message: "e".repeat(MAX_TECHNICAL_SUMMARY_CHARS + 1),
      timestamp: None,
    });
    let summary = event_summary(std::slice::from_ref(&event), 0, &event);

    assert!(summary.summary_truncated);
    assert_eq!(summary.summary.chars().count(), MAX_TECHNICAL_SUMMARY_CHARS);
    assert!(summary.summary.ends_with('…'));
  }

  #[test]
  fn page_and_detail_share_one_bounded_snapshot_until_the_source_changes() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("session.jsonl");
    std::fs::write(&source_path, "initial").unwrap();
    let locator = SessionLocator {
      version: 1,
      provider: ViewerProvider::Codex,
      session_id: "fixture".to_string(),
      source_path: source_path.clone(),
    };
    let session_key = encode_session_key(&locator).unwrap();
    let loads = Arc::new(AtomicUsize::new(0));
    let service = ViewerService::new(Arc::new(CountingRepository {
      loads: Arc::clone(&loads),
    }));

    let page = service
      .load_event_page(EventPageRequest {
        session_key: session_key.clone(),
        cursor: None,
        offset: None,
        direction: PageDirection::Forward,
        limit: None,
      })
      .unwrap();
    service
      .load_event_detail(LoadEventDetailRequest {
        session_key: session_key.clone(),
        event_key: page.events[0].event_key.clone(),
      })
      .unwrap();
    assert_eq!(loads.load(Ordering::SeqCst), 1);

    std::fs::write(&source_path, "source revision changed").unwrap();
    service
      .load_event_page(EventPageRequest {
        session_key,
        cursor: None,
        offset: None,
        direction: PageDirection::Forward,
        limit: None,
      })
      .unwrap();
    assert_eq!(loads.load(Ordering::SeqCst), 2);
  }

  #[test]
  fn hidden_event_pages_and_details_never_expose_content_or_native_data() {
    let service = service_with_session(hidden_message_session());
    let session_key = key_for("fixture");
    let page = service
      .load_event_page(EventPageRequest {
        session_key: session_key.clone(),
        cursor: None,
        offset: None,
        direction: PageDirection::Forward,
        limit: None,
      })
      .unwrap();
    let page_json = serde_json::to_string(&page).unwrap();
    assert!(page.events[0].is_hidden);
    assert!(!page_json.contains("message-secret"));
    assert!(!page_json.contains("native-secret"));

    let detail = service_with_session(hidden_message_session())
      .load_event_detail(LoadEventDetailRequest {
        session_key,
        event_key: page.events[0].event_key.clone(),
      })
      .unwrap();
    let detail_json = serde_json::to_string(&detail).unwrap();
    assert!(detail.is_hidden);
    assert!(detail.native.is_none());
    assert_eq!(detail.event["redacted"], true);
    assert!(!detail_json.contains("message-secret"));
    assert!(!detail_json.contains("native-secret"));
  }

  #[test]
  fn hidden_unknown_pi_records_are_also_redacted() {
    let hidden = AgentEvent::Unknown(UnknownEvent {
      provider: Provider::Pi,
      session_id: Some("fixture".to_string()),
      native_type: Some("custom_message".to_string()),
      native: Some(json!({"type": "custom_message", "display": false, "content": "secret"})),
      timestamp: None,
    });
    let service = service_with_session(loaded_session(vec![hidden]));
    let detail = service
      .load_event_detail(LoadEventDetailRequest {
        session_key: key_for("fixture"),
        event_key: encode_event_key(0),
      })
      .unwrap();

    assert!(detail.is_hidden);
    assert!(detail.native.is_none());
    assert!(!serde_json::to_string(&detail).unwrap().contains("secret"));
  }

  #[test]
  fn redacted_reasoning_details_never_expose_readable_or_native_payloads() {
    let event = AgentEvent::Reasoning(ReasoningEvent {
      provenance: Some(MessageProvenance {
        source: json!({"kind": "fixture"}),
        display: None,
        native: Some(json!({"native-secret": "provider-withheld native"})),
        surface_op: None,
        source_event_seqs: None,
      }),
      provider: Provider::Pi,
      session_id: Some("fixture".to_string()),
      message_id: Some("reasoning".to_string()),
      parent_id: None,
      phase: Phase::Finished,
      text: Some("provider-withheld text".to_string()),
      summary: Some("provider-withheld summary".to_string()),
      redacted: Some(true),
      encrypted_content: Some("provider-withheld encrypted content".to_string()),
      signature: Some("provider-withheld signature".to_string()),
      timestamp: None,
    });
    let detail = service_with_session(loaded_session(vec![event]))
      .load_event_detail(LoadEventDetailRequest {
        session_key: key_for("fixture"),
        event_key: encode_event_key(0),
      })
      .unwrap();

    let serialized = serde_json::to_string(&detail).unwrap();
    assert!(!detail.is_hidden);
    assert_eq!(detail.event["type"], "reasoning");
    assert_eq!(detail.event["redacted"], true);
    assert!(detail.native.is_none());
    for secret in [
      "provider-withheld text",
      "provider-withheld summary",
      "provider-withheld encrypted content",
      "provider-withheld signature",
      "provider-withheld native",
    ] {
      assert!(!serialized.contains(secret));
    }
  }

  #[test]
  fn non_hidden_detail_separates_native_from_normalized_data() {
    let event = AgentEvent::Unknown(UnknownEvent {
      provider: Provider::Codex,
      session_id: Some("fixture".to_string()),
      native_type: Some("future_event".to_string()),
      native: Some(json!({"future": true})),
      timestamp: None,
    });
    let service = service_with_session(loaded_session(vec![event]));
    let detail = service
      .load_event_detail(LoadEventDetailRequest {
        session_key: key_for("fixture"),
        event_key: encode_event_key(0),
      })
      .unwrap();

    assert_eq!(detail.native, Some(json!({"future": true})));
    assert!(detail.event.get("native").is_none());
  }

  #[test]
  fn oversized_detail_representations_are_replaced_with_bounded_json_placeholders() {
    let oversized = "x".repeat(MAX_DETAIL_VALUE_BYTES + 1);
    let event = AgentEvent::Message(MessageEvent {
      provenance: Some(MessageProvenance {
        source: json!({"kind": "fixture"}),
        display: None,
        native: Some(json!({"payload": oversized.clone()})),
        surface_op: None,
        source_event_seqs: None,
      }),
      provider: Provider::Pi,
      session_id: Some("fixture".to_string()),
      message_id: Some("oversized".to_string()),
      parent_id: None,
      role: Role::Assistant,
      delivery: MessageDelivery::Final,
      phase: Phase::Finished,
      text: oversized,
      timestamp: None,
    });
    let detail = service_with_session(loaded_session(vec![event]))
      .load_event_detail(LoadEventDetailRequest {
        session_key: key_for("fixture"),
        event_key: encode_event_key(0),
      })
      .unwrap();
    let native = detail
      .native
      .as_ref()
      .expect("native placeholder should remain available");

    assert_eq!(detail.event["truncated"], true);
    assert_eq!(detail.event["representation"], "normalized_event");
    assert!(detail.event["original_size_bytes"].as_u64().unwrap() > MAX_DETAIL_VALUE_BYTES as u64);
    assert_eq!(detail.event["limit_bytes"], MAX_DETAIL_VALUE_BYTES);
    assert_eq!(native["truncated"], true);
    assert_eq!(native["representation"], "provider_native");
    assert!(native["original_size_bytes"].as_u64().unwrap() > MAX_DETAIL_VALUE_BYTES as u64);
    assert_eq!(native["limit_bytes"], MAX_DETAIL_VALUE_BYTES);
    assert!(serde_json::to_vec(&detail).unwrap().len() < 4 * 1024);
  }

  fn service_with_session(session: LoadedSession) -> ViewerService {
    ViewerService::new(Arc::new(FakeRepository {
      listings: HashMap::new(),
      loaded: Mutex::new(Some(session)),
    }))
  }

  fn visible_session(count: usize) -> LoadedSession {
    loaded_session(
      (0..count)
        .map(|index| {
          AgentEvent::Message(MessageEvent {
            provenance: None,
            provider: Provider::Codex,
            session_id: Some("fixture".to_string()),
            message_id: Some(format!("message-{index}")),
            parent_id: None,
            role: Role::Assistant,
            delivery: MessageDelivery::Final,
            phase: Phase::Finished,
            text: format!("message {index}"),
            timestamp: None,
          })
        })
        .collect(),
    )
  }

  fn message_event(text: &str) -> AgentEvent {
    AgentEvent::Message(MessageEvent {
      provenance: None,
      provider: Provider::Codex,
      session_id: Some("fixture".to_string()),
      message_id: Some("message".to_string()),
      parent_id: None,
      role: Role::Assistant,
      delivery: MessageDelivery::Final,
      phase: Phase::Finished,
      text: text.to_string(),
      timestamp: None,
    })
  }

  fn usage_event(kind: UsageKind, provider: Provider) -> AgentEvent {
    AgentEvent::Usage(UsageEvent {
      kind,
      provider,
      session_id: Some("fixture".to_string()),
      turn_id: None,
      step_id: None,
      message_id: None,
      record_id: None,
      input_tokens: 0,
      output_tokens: 0,
      total_tokens: None,
      cache_read_tokens: None,
      cache_write_tokens: None,
      reasoning_tokens: None,
      native: json!({}),
      timestamp: None,
    })
  }

  fn reasoning_event(
    summary: Option<&str>,
    text: Option<&str>,
    encrypted_content: Option<&str>,
    signature: Option<&str>,
    redacted: Option<bool>,
  ) -> AgentEvent {
    AgentEvent::Reasoning(ReasoningEvent {
      provenance: None,
      provider: Provider::Pi,
      session_id: Some("fixture".to_string()),
      message_id: Some("reasoning".to_string()),
      parent_id: None,
      phase: Phase::Finished,
      text: text.map(str::to_string),
      summary: summary.map(str::to_string),
      redacted,
      encrypted_content: encrypted_content.map(str::to_string),
      signature: signature.map(str::to_string),
      timestamp: None,
    })
  }

  #[allow(clippy::too_many_arguments)]
  fn tool_call(
    provider: Provider,
    tool_name: &str,
    tool_call_id: &str,
    tool_kind: ToolKind,
    summary: Option<ToolSummary>,
    phase: Phase,
    output: Option<Value>,
  ) -> AgentEvent {
    let is_error = match summary.as_ref() {
      Some(ToolSummary::Shell {
        exit_code: Some(exit_code),
        ..
      }) => Some(*exit_code != 0),
      _ => None,
    };
    AgentEvent::ToolCall(ToolCallEvent {
      provider,
      session_id: Some("fixture".to_string()),
      turn_id: None,
      message_id: None,
      parent_id: None,
      record_kind: match phase {
        Phase::Started => ToolRecordKind::Invocation,
        Phase::Delta | Phase::Updated => ToolRecordKind::Progress,
        Phase::Finished if output.is_some() => ToolRecordKind::Result,
        Phase::Finished => ToolRecordKind::Snapshot,
      },
      tool_call_id: Some(tool_call_id.to_string()),
      provider_tool_name: Some(tool_name.to_string()),
      tool_name: Some(tool_name.to_string()),
      tool_kind,
      transport: Some(ToolTransport::Native),
      summary,
      phase,
      input: None,
      output,
      is_error,
      native: None,
      timestamp: None,
    })
  }

  fn with_tool_input(mut event: AgentEvent, input: Value) -> AgentEvent {
    let AgentEvent::ToolCall(tool) = &mut event else {
      panic!("fixture must be a tool event");
    };
    tool.input = Some(input);
    event
  }

  fn hidden_message_session() -> LoadedSession {
    loaded_session(vec![AgentEvent::Message(MessageEvent {
      provenance: Some(MessageProvenance {
        source: json!({"kind": "extension"}),
        display: Some(false),
        native: Some(json!({"secret": "native-secret"})),
        surface_op: None,
        source_event_seqs: None,
      }),
      provider: Provider::Pi,
      session_id: Some("fixture".to_string()),
      message_id: Some("hidden".to_string()),
      parent_id: None,
      role: Role::System,
      delivery: MessageDelivery::Unspecified,
      phase: Phase::Finished,
      text: "message-secret".to_string(),
      timestamp: None,
    })])
  }

  fn loaded_session(events: Vec<AgentEvent>) -> LoadedSession {
    LoadedSession {
      reference: session_ref("fixture", None, "/projects/fixture", "2026-06-01T00:00:00Z"),
      events,
      history_status: SessionHistoryStatus::Complete,
    }
  }

  fn key_for(session_id: &str) -> String {
    encode_session_key(&SessionLocator {
      version: 1,
      provider: ViewerProvider::Codex,
      session_id: session_id.to_string(),
      source_path: PathBuf::from("/fixtures/session.jsonl"),
    })
    .unwrap()
  }

  fn session_ref(id: &str, parent: Option<&str>, cwd: &str, timestamp: &str) -> SessionRef {
    SessionRef {
      id: id.to_string(),
      title: None,
      preview: None,
      parent_session_id: parent.map(str::to_string),
      agent_path: None,
      agent_nickname: None,
      agent_role: None,
      path: PathBuf::from(format!("/fixtures/{id}.jsonl")),
      cwd: Some(cwd.to_string()),
      timestamp: Some(timestamp.to_string()),
      message_count: 2,
    }
  }

  fn session_header(id: &str, parent: Option<&str>, cwd: &str, timestamp: &str) -> SessionHeader {
    let updated_at_ms = parse_updated_at_ms(Some(timestamp));
    SessionHeader {
      id: id.to_string(),
      title: None,
      preview: None,
      parent_session_id: parent.map(str::to_string),
      agent_path: None,
      agent_nickname: None,
      agent_role: None,
      path: PathBuf::from(format!("/fixtures/{id}.jsonl")),
      cwd: Some(cwd.to_string()),
      timestamp: Some(timestamp.to_string()),
      updated_at: Some(timestamp.to_string()),
      updated_at_ms,
    }
  }

  fn session_header_with_updated(
    id: &str,
    cwd: &str,
    timestamp: &str,
    updated_at: &str,
    updated_at_ms: i64,
  ) -> SessionHeader {
    SessionHeader {
      id: id.to_string(),
      title: None,
      preview: None,
      parent_session_id: None,
      agent_path: None,
      agent_nickname: None,
      agent_role: None,
      path: PathBuf::from(format!("/fixtures/{id}.jsonl")),
      cwd: Some(cwd.to_string()),
      timestamp: Some(timestamp.to_string()),
      updated_at: Some(updated_at.to_string()),
      updated_at_ms: Some(updated_at_ms),
    }
  }

  #[test]
  fn encoded_session_key_contains_expected_locator() {
    let key = key_for("fixture");
    let locator = decode_session_key(&key).unwrap();
    assert_eq!(locator.provider, ViewerProvider::Codex);
    assert_eq!(locator.session_id, "fixture");
  }
}
