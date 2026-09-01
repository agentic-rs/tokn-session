use std::path::PathBuf;

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokn_session_client::Source;
use tokn_session_core::SessionHistoryStatus;

pub const DEFAULT_PAGE_LIMIT: usize = 50;
pub const MAX_PAGE_LIMIT: usize = 200;
const MAX_SESSION_KEY_BYTES: usize = 64 * 1024;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewerProvider {
  Codex,
  Pi,
  #[serde(rename = "opencode")]
  OpenCode,
  Dsh,
}

impl ViewerProvider {
  pub const ALL: [Self; 4] = [Self::Codex, Self::Pi, Self::OpenCode, Self::Dsh];

  pub fn as_str(self) -> &'static str {
    match self {
      Self::Codex => "codex",
      Self::Pi => "pi",
      Self::OpenCode => "opencode",
      Self::Dsh => "dsh",
    }
  }

  pub fn source(self) -> Source {
    match self {
      Self::Codex => Source::Codex,
      Self::Pi => Source::Pi,
      Self::OpenCode => Source::OpenCode,
      Self::Dsh => Source::Dsh,
    }
  }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SessionQuery {
  #[serde(default)]
  pub providers: Vec<ViewerProvider>,
  pub search: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ListSessionsRequest {
  #[serde(default)]
  pub query: SessionQuery,
  pub cursor: Option<String>,
  pub offset: Option<usize>,
  pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ListSessionsResponse {
  pub sessions: Vec<SessionSummary>,
  pub next_cursor: Option<String>,
  pub source_errors: Vec<SourceError>,
}

/// A bounded, metadata-only page of direct descendants for one session.
///
/// The sidebar loads these on demand rather than materializing an entire
/// session family in the root listing. That keeps a single unusually broad
/// delegation tree from making the initial IPC response unbounded.
#[derive(Clone, Debug, Deserialize)]
pub struct ListSessionChildrenRequest {
  pub parent_session_key: String,
  pub cursor: Option<String>,
  pub offset: Option<usize>,
  pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ListSessionChildrenResponse {
  pub sessions: Vec<SessionSummary>,
  pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SourceError {
  pub provider: ViewerProvider,
  pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionSummary {
  pub session_key: String,
  pub session_id: String,
  pub provider: ViewerProvider,
  pub title: Option<String>,
  pub preview: Option<String>,
  pub project: Option<String>,
  pub cwd: Option<String>,
  pub updated_at_ms: Option<i64>,
  pub timestamp: Option<String>,
  pub parent_session_id: Option<String>,
  /// True only when the parent relation resolves to a canonical header in the
  /// same provider. Orphaned and cycle-broken records retain their raw parent
  /// ID but remain visible as roots.
  pub is_subagent: bool,
  pub agent_path: Option<String>,
  pub agent_nickname: Option<String>,
  pub agent_role: Option<String>,
  /// Number of direct, canonical descendants known from metadata-only
  /// discovery. It is intentionally not a runtime state or event count.
  pub child_count: usize,
  /// Unknown for metadata-only listings. Loading an event page returns the
  /// authoritative normalized event count for the selected session.
  pub message_count: Option<usize>,
  pub event_count: Option<usize>,
  pub history_status: Option<HistoryStatus>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
  Complete,
  FilteredSubagent,
  SubagentBodyUnavailable,
}

impl From<SessionHistoryStatus> for HistoryStatus {
  fn from(value: SessionHistoryStatus) -> Self {
    match value {
      SessionHistoryStatus::Complete => Self::Complete,
      SessionHistoryStatus::FilteredSubagent => Self::FilteredSubagent,
      SessionHistoryStatus::SubagentBodyUnavailable => Self::SubagentBodyUnavailable,
    }
  }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageDirection {
  #[default]
  Forward,
  Backward,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EventPageRequest {
  pub session_key: String,
  pub cursor: Option<String>,
  pub offset: Option<usize>,
  #[serde(default)]
  pub direction: PageDirection,
  pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct EventPage {
  pub events: Vec<EventSummary>,
  pub next_cursor: Option<String>,
  pub previous_cursor: Option<String>,
  pub total_events: usize,
  pub history_status: HistoryStatus,
}

#[derive(Debug, Serialize)]
pub struct EventSummary {
  pub event_key: String,
  #[serde(rename = "type")]
  pub event_type: String,
  pub provider: ViewerProvider,
  pub timestamp: Option<String>,
  pub phase: Option<String>,
  pub role: Option<String>,
  pub title: String,
  pub summary: String,
  pub summary_truncated: bool,
  pub is_hidden: bool,
  pub is_error: Option<bool>,
  pub tool: Option<ToolCardSummary>,
  pub usage: Option<UsageCardSummary>,
  pub reasoning: Option<ReasoningCardSummary>,
  /// Aggregate metadata for a synthetic, contiguous work trajectory. The
  /// individual normalized entries remain available through the trajectory
  /// page and retain their own `event.v1.*` detail keys.
  pub trajectory: Option<TrajectoryCardSummary>,
  /// Safe historical activity metadata. A target is present only when the
  /// activity's provider-native target ID resolves to a canonical direct child
  /// of the session being viewed.
  pub agent_activity: Option<AgentActivityCardSummary>,
}

/// Bounded, source-neutral presentation metadata for one collapsed run of
/// historical work. Counts describe visible base-timeline entries; one tool
/// operation can therefore represent several provider source records.
///
/// Timestamp strings come only from parseable provider event timestamps in
/// source chronology. `duration_ms` is a decimal string so a renderer never
/// loses precision when a provider reports a large interval.
#[derive(Debug, Serialize)]
pub struct TrajectoryCardSummary {
  pub event_count: usize,
  pub source_event_count: usize,
  pub reasoning_count: usize,
  pub tool_count: usize,
  pub agent_activity_count: usize,
  pub lifecycle_count: usize,
  pub usage_count: usize,
  pub error_count: usize,
  pub unknown_count: usize,
  pub started_at: Option<String>,
  pub ended_at: Option<String>,
  pub duration_ms: Option<String>,
}

/// Bounded presentation metadata for one historical agent-activity record.
///
/// `target_agent_path` is descriptive only. Navigation is exposed exclusively
/// through a verified direct-child [`SessionSummary`] in `target`.
#[derive(Clone, Debug, Serialize)]
pub struct AgentActivityCardSummary {
  pub kind: String,
  pub event_id: Option<String>,
  pub target_session_id: Option<String>,
  pub target_agent_path: Option<String>,
  pub target: Option<SessionSummary>,
}

/// Source-neutral token accounting for a usage event.
///
/// Token counts intentionally cross the IPC boundary as decimal strings:
/// JavaScript `number` cannot represent every `u64` exactly.
#[derive(Debug, Serialize)]
pub struct UsageCardSummary {
  pub kind: String,
  pub input_tokens: String,
  pub output_tokens: String,
  pub total_tokens: Option<String>,
  pub cache_read_tokens: Option<String>,
  pub cache_write_tokens: Option<String>,
  pub reasoning_tokens: Option<String>,
  pub turn_id: Option<String>,
  pub step_id: Option<String>,
}

/// Safe reasoning metadata for a collapsed event card.
///
/// Raw encrypted reasoning, signatures, and full reasoning text deliberately
/// remain out of this projection. The viewer can use the boolean flags to
/// select an appropriate disclosure state without exposing opaque payloads.
#[derive(Debug, Serialize)]
pub struct ReasoningCardSummary {
  pub preview: Option<String>,
  pub has_summary: bool,
  pub has_text: bool,
  pub has_encrypted_content: bool,
  pub is_redacted: bool,
}

#[derive(Debug, Serialize)]
pub struct ToolCardSummary {
  pub kind: String,
  pub tool_name: Option<String>,
  pub tool_call_id: Option<String>,
  /// Derived operation state, not the source record's transport phase.
  pub status: String,
  pub provider_tool_name: Option<String>,
  pub language: Option<String>,
  pub command: Option<String>,
  pub cwd: Option<String>,
  pub terminal_session_id: Option<String>,
  pub terminal_action: Option<String>,
  pub chars_len: Option<u64>,
  pub wait_ms: Option<u64>,
  pub path: Option<String>,
  pub query: Option<String>,
  pub url: Option<String>,
  pub task_title: Option<String>,
  pub exit_code: Option<i64>,
  pub bytes: Option<u64>,
  pub added: Option<u64>,
  pub removed: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LoadEventDetailRequest {
  pub session_key: String,
  pub event_key: String,
}

/// Loads a bounded page of the existing normalized entries represented by one
/// synthetic trajectory item. The trajectory key is separate from a raw event
/// key, so expanding an item never changes the detail identity of its children.
#[derive(Clone, Debug, Deserialize)]
pub struct LoadTrajectoryEventPageRequest {
  pub session_key: String,
  pub trajectory_key: String,
  pub cursor: Option<String>,
  pub offset: Option<usize>,
  #[serde(default)]
  pub direction: PageDirection,
  pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct TrajectoryEventPage {
  pub events: Vec<EventSummary>,
  pub next_cursor: Option<String>,
  pub previous_cursor: Option<String>,
  pub total_events: usize,
}

#[derive(Debug, Serialize)]
pub struct EventDetail {
  pub event_key: String,
  pub event: Value,
  pub native: Option<Value>,
  pub is_hidden: bool,
  pub tool_output: Option<ToolOutputPreview>,
}

#[derive(Debug, Serialize)]
pub struct ToolOutputPreview {
  pub sections: Vec<ToolOutputSection>,
  pub truncated: bool,
  pub original_size_bytes: usize,
  pub source_event_key: String,
}

#[derive(Debug, Serialize)]
pub struct ToolOutputSection {
  pub label: Option<String>,
  pub text: String,
  pub format: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub(crate) struct SessionLocator {
  pub version: u8,
  pub provider: ViewerProvider,
  pub session_id: String,
  pub source_path: PathBuf,
}

pub(crate) fn encode_session_key(locator: &SessionLocator) -> Result<String, String> {
  let bytes = serde_json::to_vec(locator).map_err(|error| format!("failed to encode session key: {error}"))?;
  if bytes.len() > MAX_SESSION_KEY_BYTES {
    return Err("session source identity is too large".to_string());
  }
  Ok(format!("session.v1.{}", hex_encode(&bytes)))
}

pub(crate) fn decode_session_key(key: &str) -> Result<SessionLocator, String> {
  let encoded = key
    .strip_prefix("session.v1.")
    .ok_or_else(|| "unsupported session key".to_string())?;
  let bytes = hex_decode(encoded)?;
  let locator: SessionLocator =
    serde_json::from_slice(&bytes).map_err(|_| "invalid session key payload".to_string())?;
  if locator.version != 1 || locator.session_id.is_empty() || locator.source_path.as_os_str().is_empty() {
    return Err("invalid session key payload".to_string());
  }
  Ok(locator)
}

pub(crate) fn encode_list_cursor(offset: usize) -> String {
  format!("sessions.v1.{offset:x}")
}

pub(crate) fn decode_list_cursor(cursor: &str) -> Result<usize, String> {
  decode_cursor(cursor, "sessions.v1.")
}

pub(crate) fn encode_event_cursor(offset: usize) -> String {
  format!("events.v1.{offset:x}")
}

pub(crate) fn decode_event_cursor(cursor: &str) -> Result<usize, String> {
  decode_cursor(cursor, "events.v1.")
}

pub(crate) fn encode_event_key(index: usize) -> String {
  format!("event.v1.{index:x}")
}

pub(crate) fn decode_event_key(key: &str) -> Result<usize, String> {
  decode_cursor(key, "event.v1.")
}

/// A synthetic trajectory identity is intentionally distinct from the stable
/// source-event identity. Its numeric payload is the final source position of
/// the collapsed run, not an `event.v1.*` alias.
pub(crate) fn encode_trajectory_key(anchor: usize) -> String {
  format!("trajectory.v1.{anchor:x}")
}

pub(crate) fn decode_trajectory_key(key: &str) -> Result<usize, String> {
  decode_cursor(key, "trajectory.v1.")
}

pub(crate) fn encode_trajectory_event_cursor(anchor: usize, offset: usize) -> String {
  format!("trajectory-events.v1.{anchor:x}.{offset:x}")
}

pub(crate) fn decode_trajectory_event_cursor(cursor: &str) -> Result<(usize, usize), String> {
  let encoded = cursor
    .strip_prefix("trajectory-events.v1.")
    .filter(|value| !value.is_empty())
    .ok_or_else(|| "invalid trajectory pagination cursor".to_string())?;
  let (anchor, offset) = encoded
    .split_once('.')
    .filter(|(anchor, offset)| !anchor.is_empty() && !offset.is_empty())
    .ok_or_else(|| "invalid trajectory pagination cursor".to_string())?;
  if offset.contains('.') {
    return Err("invalid trajectory pagination cursor".to_string());
  }
  let anchor = usize::from_str_radix(anchor, 16).map_err(|_| "invalid trajectory pagination cursor".to_string())?;
  let offset = usize::from_str_radix(offset, 16).map_err(|_| "invalid trajectory pagination cursor".to_string())?;
  Ok((anchor, offset))
}

pub(crate) fn bounded_limit(limit: Option<usize>) -> Result<usize, String> {
  let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT);
  if limit == 0 {
    return Err("limit must be greater than zero".to_string());
  }
  Ok(limit.min(MAX_PAGE_LIMIT))
}

pub(crate) fn requested_offset(
  cursor: Option<&str>,
  offset: Option<usize>,
  decode: fn(&str) -> Result<usize, String>,
) -> Result<Option<usize>, String> {
  match (cursor, offset) {
    (Some(_), Some(_)) => Err("cursor and offset cannot be used together".to_string()),
    (Some(cursor), None) => decode(cursor).map(Some),
    (None, offset) => Ok(offset),
  }
}

pub(crate) fn parse_updated_at_ms(timestamp: Option<&str>) -> Option<i64> {
  let timestamp = timestamp?.trim();
  if timestamp.is_empty() {
    return None;
  }
  timestamp
    .parse::<i64>()
    .ok()
    .or_else(|| {
      DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|value| value.timestamp_millis())
    })
    .filter(|value| (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(value))
}

fn decode_cursor(cursor: &str, prefix: &str) -> Result<usize, String> {
  let encoded = cursor
    .strip_prefix(prefix)
    .filter(|value| !value.is_empty())
    .ok_or_else(|| "invalid pagination cursor".to_string())?;
  usize::from_str_radix(encoded, 16).map_err(|_| "invalid pagination cursor".to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
  const HEX: &[u8; 16] = b"0123456789abcdef";
  let mut encoded = String::with_capacity(bytes.len() * 2);
  for byte in bytes {
    encoded.push(HEX[(byte >> 4) as usize] as char);
    encoded.push(HEX[(byte & 0x0f) as usize] as char);
  }
  encoded
}

fn hex_decode(value: &str) -> Result<Vec<u8>, String> {
  if value.len() & 1 == 1 || value.len() / 2 > MAX_SESSION_KEY_BYTES {
    return Err("invalid session key encoding".to_string());
  }
  value
    .as_bytes()
    .chunks_exact(2)
    .map(|pair| {
      let high = hex_nibble(pair[0]).ok_or_else(|| "invalid session key encoding".to_string())?;
      let low = hex_nibble(pair[1]).ok_or_else(|| "invalid session key encoding".to_string())?;
      Ok(high << 4 | low)
    })
    .collect()
}

fn hex_nibble(value: u8) -> Option<u8> {
  match value {
    b'0'..=b'9' => Some(value - b'0'),
    b'a'..=b'f' => Some(value - b'a' + 10),
    b'A'..=b'F' => Some(value - b'A' + 10),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn session_keys_round_trip_source_identity() {
    let locator = SessionLocator {
      version: 1,
      provider: ViewerProvider::OpenCode,
      session_id: "session/a".to_string(),
      source_path: PathBuf::from("/stores/one/opencode.db"),
    };

    let key = encode_session_key(&locator).expect("key should encode");

    assert!(!key.contains("session/a"));
    assert_eq!(decode_session_key(&key).unwrap(), locator);
  }

  #[test]
  fn keys_reject_unknown_versions_and_malformed_payloads() {
    assert!(decode_session_key("session.v2.00").is_err());
    assert!(decode_session_key("session.v1.not-hex").is_err());

    let locator = SessionLocator {
      version: 2,
      provider: ViewerProvider::Pi,
      session_id: "session".to_string(),
      source_path: PathBuf::from("/tmp/session.jsonl"),
    };
    assert!(decode_session_key(&encode_session_key(&locator).unwrap()).is_err());
  }

  #[test]
  fn timestamps_retain_provider_milliseconds_and_parse_rfc3339() {
    assert_eq!(parse_updated_at_ms(Some("1787157590000")), Some(1_787_157_590_000));
    assert_eq!(
      parse_updated_at_ms(Some("2026-06-04T00:00:00Z")),
      Some(1_780_531_200_000)
    );
    assert_eq!(parse_updated_at_ms(Some("not-a-time")), None);
    assert_eq!(parse_updated_at_ms(Some("9007199254740992")), None);
  }

  #[test]
  fn provider_wire_names_match_the_frontend_contract() {
    for (provider, wire_name) in [
      (ViewerProvider::Codex, "codex"),
      (ViewerProvider::Pi, "pi"),
      (ViewerProvider::OpenCode, "opencode"),
      (ViewerProvider::Dsh, "dsh"),
    ] {
      assert_eq!(serde_json::to_value(provider).unwrap(), wire_name);
      assert_eq!(
        serde_json::from_value::<ViewerProvider>(wire_name.into()).unwrap(),
        provider
      );
    }
  }

  #[test]
  fn limit_is_bounded_and_cursor_cannot_mix_with_offset() {
    assert_eq!(bounded_limit(Some(MAX_PAGE_LIMIT + 1)).unwrap(), MAX_PAGE_LIMIT);
    assert!(bounded_limit(Some(0)).is_err());
    assert!(requested_offset(Some("sessions.v1.1"), Some(1), decode_list_cursor).is_err());
  }

  #[test]
  fn trajectory_keys_and_cursors_are_separate_from_raw_event_keys() {
    let key = encode_trajectory_key(42);
    assert_eq!(key, "trajectory.v1.2a");
    assert_eq!(decode_trajectory_key(&key).unwrap(), 42);
    assert!(decode_event_key(&key).is_err());

    let cursor = encode_trajectory_event_cursor(42, 7);
    assert_eq!(decode_trajectory_event_cursor(&cursor).unwrap(), (42, 7));
    assert!(decode_trajectory_event_cursor("trajectory-events.v1.2a").is_err());
    assert!(decode_trajectory_event_cursor("trajectory-events.v1.2a.7.extra").is_err());
  }
}
