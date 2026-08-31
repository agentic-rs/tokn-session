use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::Serialize;
use serde_json::{Value, json};
use tokn_session_client::SessionHeader;
use tokn_session_core::{AgentEvent, LifecycleOutcome, LoadedSession, Provider, Role};
use tokn_session_render::render_event_summary;

use crate::model::{
  EventDetail, EventPage, EventPageRequest, EventSummary, ListSessionsRequest, ListSessionsResponse,
  LoadEventDetailRequest, PageDirection, SessionLocator, SessionSummary, SourceError, ViewerProvider, bounded_limit,
  decode_event_cursor, decode_event_key, decode_list_cursor, decode_session_key, encode_event_cursor, encode_event_key,
  encode_list_cursor, encode_session_key, parse_updated_at_ms, requested_offset,
};
use crate::repository::{NativeRepository, ViewerRepository};

const MAX_DETAIL_VALUE_BYTES: usize = 512 * 1024;
const MAX_MESSAGE_SUMMARY_CHARS: usize = 16 * 1024;
const MAX_TECHNICAL_SUMMARY_CHARS: usize = 500;

#[derive(Clone)]
pub(crate) struct ViewerService {
  repository: Arc<dyn ViewerRepository>,
  loaded_session_cache: Arc<Mutex<Option<CachedSession>>>,
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

impl ViewerService {
  pub fn native() -> Self {
    Self {
      repository: Arc::new(NativeRepository),
      loaded_session_cache: Arc::new(Mutex::new(None)),
    }
  }

  #[cfg(test)]
  fn new(repository: Arc<dyn ViewerRepository>) -> Self {
    Self {
      repository,
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
    let mut sessions = Vec::new();
    let mut source_errors = Vec::new();

    for provider in providers {
      let headers = match self.repository.list_session_headers(provider) {
        Ok(headers) => headers,
        Err(message) => {
          source_errors.push(SourceError { provider, message });
          continue;
        }
      };

      let mut conversion_error = None;
      for header in headers {
        // The global roster intentionally shows roots. Descendants will be
        // loaded within a selected root's conversation tree in a later slice.
        if header.parent_session_id.is_some() {
          continue;
        }
        match session_summary(provider, header) {
          Ok(summary) if matches_search(&summary, search.as_deref()) => sessions.push(summary),
          Ok(_) => {}
          Err(error) => {
            conversion_error.get_or_insert(error);
          }
        };
      }
      if let Some(message) = conversion_error {
        source_errors.push(SourceError { provider, message });
      }
    }

    sessions.sort_by(|left, right| {
      right
        .updated_at_ms
        .cmp(&left.updated_at_ms)
        .then_with(|| right.timestamp.cmp(&left.timestamp))
        .then_with(|| left.provider.as_str().cmp(right.provider.as_str()))
        .then_with(|| left.session_id.cmp(&right.session_id))
        .then_with(|| left.session_key.cmp(&right.session_key))
    });

    let start = offset.min(sessions.len());
    let end = start.saturating_add(limit).min(sessions.len());
    let next_cursor = (end < sessions.len()).then(|| encode_list_cursor(end));
    let sessions = sessions[start..end].to_vec();

    Ok(ListSessionsResponse {
      sessions,
      next_cursor,
      source_errors,
    })
  }

  pub fn load_event_page(&self, request: EventPageRequest) -> Result<EventPage, String> {
    let limit = bounded_limit(request.limit)?;
    let locator = decode_session_key(&request.session_key)?;
    let loaded = self.load_verified(&locator)?;
    let total_events = loaded.events.len();
    let requested = requested_offset(request.cursor.as_deref(), request.offset, decode_event_cursor)?;
    let boundary = requested.unwrap_or(match request.direction {
      PageDirection::Forward => 0,
      PageDirection::Backward => total_events,
    });
    let (start, end) = event_page_bounds(total_events, boundary, request.direction, limit)?;
    let events = loaded.events[start..end]
      .iter()
      .enumerate()
      .map(|(relative_index, event)| event_summary(start + relative_index, event))
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
    let index = decode_event_key(&request.event_key)?;
    let loaded = self.load_verified(&locator)?;
    let event = loaded
      .events
      .get(index)
      .ok_or_else(|| "event key is outside the session".to_string())?;
    let is_hidden = event.is_hidden();
    if is_hidden {
      return Ok(EventDetail {
        event_key: request.event_key,
        event: json!({
          "type": normalized_event_type(event),
          "provider": provider_for_event(event).as_str(),
          "redacted": true,
        }),
        native: None,
        is_hidden: true,
      });
    }

    let native = native_detail(event)
      .map(|value| bounded_detail_value(value, "provider_native"))
      .transpose()?;
    let mut normalized =
      serde_json::to_value(event).map_err(|error| format!("failed to serialize normalized event: {error}"))?;
    remove_embedded_native(&mut normalized);
    let normalized = bounded_detail_value(normalized, "normalized_event")?;
    Ok(EventDetail {
      event_key: request.event_key,
      event: normalized,
      native,
      is_hidden: false,
    })
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

fn session_summary(provider: ViewerProvider, header: SessionHeader) -> Result<SessionSummary, String> {
  let locator = SessionLocator {
    version: 1,
    provider,
    session_id: header.id.clone(),
    source_path: header.path.clone(),
  };
  let project = header.cwd.as_deref().and_then(path_name).map(str::to_string);
  let title = header
    .agent_nickname
    .as_deref()
    .or(header.agent_path.as_deref())
    .map(str::to_string)
    .unwrap_or_else(|| header.id.clone());
  Ok(SessionSummary {
    session_key: encode_session_key(&locator)?,
    session_id: header.id,
    provider,
    title,
    project,
    cwd: header.cwd,
    updated_at_ms: header
      .updated_at_ms
      .or_else(|| parse_updated_at_ms(header.updated_at.as_deref())),
    timestamp: header.timestamp,
    parent_session_id: header.parent_session_id,
    agent_path: header.agent_path,
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
    Some(session.title.as_str()),
    session.project.as_deref(),
    session.cwd.as_deref(),
    session.agent_path.as_deref(),
  ]
  .into_iter()
  .flatten()
  .any(|value| value.to_lowercase().contains(search))
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

fn event_summary(index: usize, event: &AgentEvent) -> EventSummary {
  let hidden = event.is_hidden();
  let title = if hidden {
    "Hidden provider content".to_string()
  } else {
    truncate(event_title(event), 120)
  };
  let summary = if hidden {
    render_event_summary(event)
  } else {
    match event {
      AgentEvent::Message(message) => message.text.clone(),
      AgentEvent::Reasoning(reasoning) => reasoning
        .summary
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
          reasoning
            .text
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
        })
        .unwrap_or_else(|| render_event_summary(event)),
      _ => render_event_summary(event),
    }
  };
  let summary_max_chars = if !hidden && matches!(event, AgentEvent::Message(_)) {
    MAX_MESSAGE_SUMMARY_CHARS
  } else {
    MAX_TECHNICAL_SUMMARY_CHARS
  };
  let (summary, summary_truncated) = truncate_with_flag(summary, summary_max_chars);
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
  use std::sync::Mutex;
  use std::sync::atomic::{AtomicUsize, Ordering};

  use serde_json::json;
  use tokn_session_core::{
    ErrorEvent, MessageDelivery, MessageEvent, MessageProvenance, Phase, ReasoningEvent, SessionHistoryStatus,
    SessionRef, ToolCallEvent, ToolKind, UnknownEvent,
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

    fn load_session(&self, _locator: &SessionLocator) -> Result<LoadedSession, String> {
      self.loads.fetch_add(1, Ordering::SeqCst);
      Ok(visible_session(1))
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
      message_id: None,
      parent_id: None,
      tool_call_id: Some("call-1".to_string()),
      tool_name: Some("shell".to_string()),
      tool_kind: ToolKind::Shell,
      summary: None,
      phase: Phase::Finished,
      input: None,
      output: None,
      is_error: Some(false),
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
  fn reasoning_summaries_preserve_multiline_markdown() {
    let markdown = "## Approach\n\n- inspect the source\n- verify the result\n";
    let summary = event_summary(
      0,
      &AgentEvent::Reasoning(ReasoningEvent {
        provenance: None,
        provider: Provider::Codex,
        session_id: Some("fixture".to_string()),
        message_id: Some("reasoning".to_string()),
        parent_id: None,
        phase: Phase::Finished,
        text: Some("Longer private reasoning".to_string()),
        summary: Some(markdown.to_string()),
        encrypted_content: None,
        signature: None,
        timestamp: None,
      }),
    );

    assert_eq!(summary.summary, markdown);
    assert!(!summary.summary_truncated);
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
    let summary = event_summary(0, &event);

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
