export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };

export const PROVIDERS = ["codex", "pi", "opencode", "zcode", "workbuddy", "dsh"] as const;

export type ViewerProvider = (typeof PROVIDERS)[number];

export type EventType =
  | "session_started"
  | "provider_changed"
  | "session_settings_applied"
  | "message"
  | "reasoning"
  | "goal_updated"
  | "agent_activity"
  | "tool_call"
  | "lifecycle"
  | "usage"
  | "metadata"
  | "error"
  | "unknown"
  /** A projected whole-turn timeline entry with lazily loaded source events. */
  | "trajectory";

export type EventPhase = "started" | "delta" | "updated" | "finished";
export type MessageRole = "user" | "assistant" | "system" | "tool" | "unknown";

export type ToolKind =
  | "code_execution"
  | "shell"
  | "terminal"
  | "file_read"
  | "file_write"
  | "file_edit"
  | "search"
  | "web"
  | "task"
  | "unknown";

export type ToolOperationStatus = "pending" | "running" | "completed" | "failed";

export interface ToolCardSummary {
  kind: ToolKind | string;
  tool_name: string | null;
  tool_call_id: string | null;
  /** Derived operation state; absent only when connected to an older backend. */
  status?: ToolOperationStatus | string;
  provider_tool_name?: string | null;
  language?: string | null;
  command: string | null;
  cwd: string | null;
  terminal_session_id?: string | null;
  terminal_action?: "send" | "wait" | string | null;
  chars_len?: number | null;
  wait_ms?: number | null;
  path: string | null;
  query: string | null;
  url: string | null;
  task_title: string | null;
  exit_code: number | null;
  bytes: number | null;
  added: number | null;
  removed: number | null;
}

/**
 * Token counters cross IPC as decimal strings so Rust u64 values retain their
 * exact value in a JavaScript renderer.
 */
export interface UsageCardSummary {
  kind: string;
  input_tokens: string;
  output_tokens: string;
  total_tokens: string | null;
  cache_read_tokens: string | null;
  cache_write_tokens: string | null;
  reasoning_tokens: string | null;
  turn_id: string | null;
  step_id: string | null;
}

/** Safe presentation metadata only; detailed reasoning stays behind event detail. */
export interface ReasoningCardSummary {
  preview: string | null;
  has_summary: boolean;
  has_text: boolean;
  has_encrypted_content: boolean;
  is_redacted: boolean;
}

export type ToolOutputFormat = "text" | "json";

export interface ToolOutputSection {
  label: string | null;
  text: string;
  format: ToolOutputFormat;
}

export interface ToolOutputPreview {
  sections: ToolOutputSection[];
  truncated: boolean;
  original_size_bytes: number;
  source_event_key: string;
}

export type SessionHistoryStatus =
  | "complete"
  | "filtered_subagent"
  | "subagent_body_unavailable";

export interface SessionSummary {
  session_key: string;
  session_id: string;
  parent_session_id: string | null;
  /** True only when the source-neutral parent link resolved safely. */
  is_subagent: boolean;
  provider: ViewerProvider;
  title: string | null;
  preview: string | null;
  project: string | null;
  cwd: string | null;
  updated_at_ms: number | null;
  timestamp: string | null;
  agent_path: string | null;
  agent_nickname: string | null;
  agent_role: string | null;
  /** Direct descendants discovered from headers; this is not runtime status. */
  child_count: number;
  /** Null for metadata-only listings; event pages provide total_events. */
  message_count: number | null;
  event_count: number | null;
  history_status: SessionHistoryStatus | null;
  /** True when this session has a visible message the viewer has not opened. */
  has_unread: boolean;
  /** True when a known descendant has unread visible activity. */
  has_unread_descendant?: boolean;
}

export interface SessionListQuery {
  providers?: ViewerProvider[];
  search?: string;
}

export interface ListSessionsRequest {
  query: SessionListQuery;
  cursor?: string;
  offset?: number;
  limit?: number;
}

export interface SourceError {
  provider: ViewerProvider;
  message: string;
}

export interface ListSessionsResponse {
  sessions: SessionSummary[];
  next_cursor: string | null;
  source_errors: SourceError[];
  /** Selected provider catalogs that have not completed their first durable index pass. */
  pending_providers: ViewerProvider[];
}

export interface ListSessionChildrenRequest {
  parent_session_key: string;
  cursor?: string;
  offset?: number;
  limit?: number;
}

export interface ListSessionChildrenResponse {
  sessions: SessionSummary[];
  next_cursor: string | null;
}

/**
 * Metadata-only signal emitted after a durable sidebar index refresh.
 * The keys identify only sessions with newly eligible visible messages; they
 * let an already open matching timeline refresh without disturbing one that
 * belongs to an unrelated provider or source.
 */
export interface SessionIndexChangedEvent {
  changed: boolean;
  attention_session_keys: string[];
}

/** Local sidebar state for one lazily loaded direct-child page sequence. */
export interface SessionChildrenState {
  sessions: SessionSummary[];
  next_cursor: string | null;
  is_loading: boolean;
  is_loading_more: boolean;
  error: string | null;
}

/**
 * Compact, historical delegation metadata for an `agent_activity` event.
 *
 * `target` is present only when the backend can prove that the activity
 * points at a known direct child in the same provider. It is deliberately not
 * a live subagent-state assertion.
 */
export interface AgentActivityCardSummary {
  kind: string;
  event_id: string | null;
  target_session_id: string | null;
  target_agent_path: string | null;
  target: SessionSummary | null;
}

/**
 * Compact aggregate metadata for a projected whole-turn trajectory.
 *
 * `duration_ms` stays a decimal string because the backend may calculate a
 * duration larger than JavaScript can represent exactly as a Number.
 */
export interface TrajectoryCardSummary {
  event_count: number;
  tool_count: number;
  reasoning_count: number;
  agent_activity_count: number;
  error_count: number;
  unknown_count: number;
  started_at: string | null;
  ended_at: string | null;
  duration_ms: string | null;
}

export interface EventSummary {
  event_key: string;
  type: EventType | string;
  provider: ViewerProvider;
  timestamp: string | null;
  phase: EventPhase | string | null;
  role: MessageRole | string | null;
  title: string;
  summary: string;
  summary_truncated: boolean;
  is_hidden: boolean;
  is_error: boolean | null;
  /** Optional while the viewer remains compatible with older backends. */
  agent_activity?: AgentActivityCardSummary | null;
  /** Present only for a projected whole-turn timeline entry. */
  trajectory?: TrajectoryCardSummary | null;
  tool: ToolCardSummary | null;
  usage: UsageCardSummary | null;
  reasoning: ReasoningCardSummary | null;
}

export type EventPageDirection = "forward" | "backward";

export interface LoadEventPageRequest {
  session_key: string;
  cursor?: string;
  offset?: number;
  direction?: EventPageDirection;
  limit?: number;
}

export interface AcknowledgeSessionAttentionRequest {
  session_key: string;
  /** Decimal string to retain an arbitrary SQLite revision exactly. */
  attention_revision: string;
}

export interface AcknowledgeSessionAttentionResponse {
  changed: boolean;
}

export interface EventPageResponse {
  events: EventSummary[];
  next_cursor: string | null;
  previous_cursor: string | null;
  total_events: number;
  history_status: SessionHistoryStatus;
  /** Opaque indexed snapshot; absent while connected to an older backend. */
  attention_revision?: string | null;
}

export interface LoadTrajectoryEventPageRequest {
  session_key: string;
  trajectory_key: string;
  cursor?: string;
  direction?: EventPageDirection;
  limit?: number;
}

export interface TrajectoryEventPageResponse {
  events: EventSummary[];
  next_cursor: string | null;
  previous_cursor: string | null;
  total_events: number;
}

export type TrajectoryPageLoadDirection = "initial" | "older" | "newer";

/**
 * One locally cached, bounded source-event sequence for a whole-turn card.
 * It is intentionally separate from the parent session's event page state.
 */
export interface TrajectoryEventPageState {
  events: EventSummary[];
  next_cursor: string | null;
  previous_cursor: string | null;
  total_events: number | null;
  has_loaded: boolean;
  is_loading: boolean;
  is_loading_older: boolean;
  is_loading_newer: boolean;
  error: string | null;
  error_direction: TrajectoryPageLoadDirection | null;
  error_cursor: string | null;
}

export interface LoadEventDetailRequest {
  session_key: string;
  event_key: string;
}

export interface EventDetail {
  event_key: string;
  event: JsonValue;
  native: JsonValue | null;
  is_hidden: boolean;
  tool_output: ToolOutputPreview | null;
}

export interface AsyncState<T> {
  data: T;
  error: string | null;
  is_loading: boolean;
}
