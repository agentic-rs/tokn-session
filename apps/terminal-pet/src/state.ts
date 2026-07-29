import {
  asNumber,
  asObject,
  asString,
  type AgentEvent,
  type JsonObject,
  type RelayEvent,
  type RelaySession
} from "./protocol";

export const PET_STATES = [
  "idle",
  "running",
  "needs_input",
  "ready",
  "blocked"
] as const;

export type PetState = typeof PET_STATES[number];

export interface PetPolicy {
  ready_debounce_ms: number;
  ready_hold_ms: number;
  recent_completion_ms: number;
  error_grace_ms: number;
  running_lease_ms: number;
  open_tool_lease_ms: number;
  blocked_lease_ms: number;
  input_lease_ms: number;
}

export interface PetFocus {
  topic: string;
  state: PetState;
  state_changed_at: number;
  last_event_at: number;
  label: string;
  provider?: string;
  project_label?: string;
  title?: string;
  session_id: string;
  agent?: string;
  completed_at?: number;
  recently_completed: boolean;
}

export interface PetSnapshot {
  state: PetState;
  state_changed_at: number;
  active_sessions: number;
  total_sessions: number;
  sessions: PetFocus[];
  focus?: PetFocus;
}

interface SessionActivity {
  topic: string;
  session: RelaySession;
  last_event_at: number;
  label: string;
  running_until: number;
  ready_after?: number;
  ready_until?: number;
  completed_at?: number;
  blocked_after?: number;
  blocked_until?: number;
  open_tools: Map<string, number>;
  pending_interactions: Map<string, number>;
}

const DEFAULT_POLICY: PetPolicy = {
  ready_debounce_ms: 750,
  ready_hold_ms: 30_000,
  recent_completion_ms: 5 * 60_000,
  error_grace_ms: 1_500,
  running_lease_ms: 3 * 60_000,
  open_tool_lease_ms: 30 * 60_000,
  blocked_lease_ms: 60 * 60_000,
  input_lease_ms: 30 * 60_000
};

const INPUT_NATIVE_TYPES = new Set([
  "event_msg.exec_approval_request",
  "event_msg.apply_patch_approval_request",
  "event_msg.request_permissions",
  "event_msg.request_user_input",
  "event_msg.elicitation_request",
  "event_msg.model_verification"
]);

const RUNNING_NATIVE_TYPES = new Set([
  "event_msg.task_started",
  "event_msg.turn_started"
]);

const READY_NATIVE_TYPES = new Set([
  "event_msg.task_complete",
  "event_msg.turn_complete"
]);

const BLOCKED_NATIVE_TYPES = new Set([
  "event_msg.turn_aborted"
]);

const STATE_PRIORITY: Record<PetState, number> = {
  idle: 0,
  running: 1,
  ready: 2,
  blocked: 3,
  needs_input: 4
};

export class PetStore {
  readonly policy: PetPolicy;
  readonly sessions = new Map<string, SessionActivity>();

  constructor(policy: Partial<PetPolicy> = {}) {
    this.policy = {
      ...DEFAULT_POLICY,
      ...policy
    };
  }

  ingest(relay: RelayEvent, nowMs = Date.now()): void {
    const activity = this.sessions.get(relay.topic) ?? newSessionActivity(relay, nowMs);
    activity.session = relay.session;
    activity.last_event_at = nowMs;
    activity.label = describeEvent(relay.event);
    this.#expireLeases(activity, nowMs);
    this.#applyEvent(activity, relay.event, nowMs);
    this.sessions.set(relay.topic, activity);
  }

  snapshot(nowMs = Date.now()): PetSnapshot {
    const candidates = [...this.sessions.values()].map((activity) => {
      this.#expireLeases(activity, nowMs);
      return this.#focusFor(activity, nowMs);
    });
    const active = candidates
      .filter((candidate) => candidate.state !== "idle")
      .sort(compareActiveSessions);
    const recent = candidates
      .filter((candidate) => candidate.state === "idle" && candidate.recently_completed)
      .sort(compareRecentCompletions);
    const sessions = [...active, ...recent];
    const focus = sessions[0];
    return {
      state: focus?.state ?? "idle",
      state_changed_at: focus?.state_changed_at ?? nowMs,
      active_sessions: active.length,
      total_sessions: candidates.length,
      sessions,
      focus
    };
  }

  acknowledge(topic: string): void {
    const activity = this.sessions.get(topic);
    if (!activity) {
      return;
    }
    activity.ready_after = undefined;
    activity.ready_until = undefined;
    activity.completed_at = undefined;
    activity.blocked_after = undefined;
    activity.blocked_until = undefined;
  }

  #applyEvent(activity: SessionActivity, event: AgentEvent, nowMs: number): void {
    switch (event.type) {
      case "session_started":
        activity.running_until = nowMs;
        activity.completed_at = undefined;
        return;
      case "message":
        this.#applyMessage(activity, event, nowMs);
        return;
      case "reasoning":
        this.#markProgress(activity, nowMs);
        return;
      case "tool_call":
        this.#applyToolCall(activity, event, nowMs);
        return;
      case "error":
        activity.ready_after = undefined;
        activity.ready_until = undefined;
        activity.completed_at = undefined;
        activity.blocked_after = nowMs + this.policy.error_grace_ms;
        activity.blocked_until = activity.blocked_after + this.policy.blocked_lease_ms;
        return;
      case "goal_updated":
        this.#applyGoal(activity, event, nowMs);
        return;
      case "agent_activity":
        this.#markProgress(activity, nowMs);
        return;
      case "unknown":
        this.#applyUnknown(activity, event, nowMs);
        return;
      case "provider_changed":
      case "session_settings_applied":
        return;
      default:
        return;
    }
  }

  #applyMessage(activity: SessionActivity, event: AgentEvent, nowMs: number): void {
    const role = asString(event.role);
    const phase = asString(event.phase);
    if (role === "assistant" && phase === "finished") {
      activity.blocked_after = undefined;
      activity.blocked_until = undefined;
      activity.ready_after = nowMs + this.policy.ready_debounce_ms;
      activity.ready_until = activity.ready_after + this.policy.ready_hold_ms;
      activity.completed_at = undefined;
      activity.running_until = activity.ready_after;
      return;
    }

    this.#markProgress(activity, nowMs);
    if (role === "user") {
      activity.open_tools.clear();
      activity.pending_interactions.clear();
    }
  }

  #applyToolCall(activity: SessionActivity, event: AgentEvent, nowMs: number): void {
    const toolName = asString(event.tool_name);
    const toolCallId = asString(event.tool_call_id);
    const phase = asString(event.phase);
    const hasInput = event.input !== undefined && event.input !== null;
    const hasOutput = event.output !== undefined && event.output !== null;

    if (toolName && isInputTool(toolName) && !hasOutput) {
      this.#markNeedsInput(activity, event, nowMs, `tool:${toolName}`);
      return;
    }

    this.#markProgress(activity, nowMs);
    if (!toolCallId) {
      return;
    }

    if (hasOutput) {
      activity.open_tools.delete(toolCallId);
      activity.pending_interactions.delete(toolCallId);
    } else if (phase === "started" || phase === "delta" || phase === "updated" || hasInput) {
      activity.open_tools.set(toolCallId, nowMs + this.policy.open_tool_lease_ms);
    } else if (phase === "finished") {
      activity.open_tools.delete(toolCallId);
    }
  }

  #applyGoal(activity: SessionActivity, event: AgentEvent, nowMs: number): void {
    const goal = asObject(event.goal);
    const status = asString(goal?.status)?.toLowerCase();
    switch (status) {
      case "complete":
      case "completed":
        this.#markReady(activity, nowMs);
        return;
      case "blocked":
      case "budget_limited":
      case "usage_limited":
        this.#markBlocked(activity, nowMs);
        return;
      case "paused":
        this.#clearTransientState(activity, nowMs);
        return;
      case "active":
      case "in_progress":
        this.#markProgress(activity, nowMs);
        return;
      default:
        this.#markProgress(activity, nowMs);
    }
  }

  #applyUnknown(activity: SessionActivity, event: AgentEvent, nowMs: number): void {
    const nativeType = asString(event.native_type)?.toLowerCase();
    if (!nativeType) {
      return;
    }
    if (INPUT_NATIVE_TYPES.has(nativeType)) {
      this.#markNeedsInput(activity, event, nowMs, nativeType);
    } else if (RUNNING_NATIVE_TYPES.has(nativeType)) {
      this.#markProgress(activity, nowMs);
    } else if (READY_NATIVE_TYPES.has(nativeType)) {
      this.#markReady(activity, nowMs);
    } else if (BLOCKED_NATIVE_TYPES.has(nativeType)) {
      this.#markBlocked(activity, nowMs);
    }
  }

  #markProgress(activity: SessionActivity, nowMs: number): void {
    activity.running_until = nowMs + this.policy.running_lease_ms;
    activity.ready_after = undefined;
    activity.ready_until = undefined;
    activity.completed_at = undefined;
    activity.blocked_after = undefined;
    activity.blocked_until = undefined;
  }

  #markReady(activity: SessionActivity, nowMs: number): void {
    activity.open_tools.clear();
    activity.pending_interactions.clear();
    activity.blocked_after = undefined;
    activity.blocked_until = undefined;
    activity.ready_after = nowMs;
    activity.ready_until = nowMs + this.policy.ready_hold_ms;
    activity.completed_at = undefined;
    activity.running_until = nowMs;
  }

  #markBlocked(activity: SessionActivity, nowMs: number): void {
    activity.ready_after = undefined;
    activity.ready_until = undefined;
    activity.completed_at = undefined;
    activity.blocked_after = nowMs;
    activity.blocked_until = nowMs + this.policy.blocked_lease_ms;
    activity.running_until = nowMs;
  }

  #markNeedsInput(
    activity: SessionActivity,
    event: AgentEvent,
    nowMs: number,
    fallbackKey: string
  ): void {
    const native = asObject(event.native);
    const interactionKey = [
      native?.approval_id,
      native?.call_id,
      native?.id,
      native?.turn_id,
      event.tool_call_id
    ].map(asString).find(Boolean) ?? fallbackKey;
    const autoResolutionMs = asNumber(native?.autoResolutionMs)
      ?? asNumber(native?.auto_resolution_ms);
    const lease = autoResolutionMs === undefined
      ? this.policy.input_lease_ms
      : autoResolutionMs + 2_000;

    activity.pending_interactions.set(interactionKey, nowMs + lease);
    activity.ready_after = undefined;
    activity.ready_until = undefined;
    activity.completed_at = undefined;
    activity.blocked_after = undefined;
    activity.blocked_until = undefined;
    activity.running_until = Math.max(activity.running_until, nowMs);
  }

  #clearTransientState(activity: SessionActivity, nowMs: number): void {
    activity.open_tools.clear();
    activity.pending_interactions.clear();
    activity.ready_after = undefined;
    activity.ready_until = undefined;
    activity.completed_at = undefined;
    activity.blocked_after = undefined;
    activity.blocked_until = undefined;
    activity.running_until = nowMs;
  }

  #expireLeases(activity: SessionActivity, nowMs: number): void {
    for (const [key, expiresAt] of activity.open_tools) {
      if (expiresAt <= nowMs) {
        activity.open_tools.delete(key);
      }
    }
    for (const [key, expiresAt] of activity.pending_interactions) {
      if (expiresAt <= nowMs) {
        activity.pending_interactions.delete(key);
      }
    }
    if (
      activity.completed_at === undefined
      && activity.ready_after !== undefined
      && activity.ready_after <= nowMs
      && activity.open_tools.size === 0
      && activity.pending_interactions.size === 0
    ) {
      activity.completed_at = activity.ready_after;
    }
    if (activity.ready_until !== undefined && activity.ready_until <= nowMs) {
      activity.ready_after = undefined;
      activity.ready_until = undefined;
    }
    if (activity.blocked_until !== undefined && activity.blocked_until <= nowMs) {
      activity.blocked_after = undefined;
      activity.blocked_until = undefined;
    }
  }

  #focusFor(activity: SessionActivity, nowMs: number): PetFocus {
    let state: PetState = "idle";
    let changedAt = activity.last_event_at;

    if (activity.pending_interactions.size > 0) {
      state = "needs_input";
    } else if (
      activity.blocked_after !== undefined
      && activity.blocked_until !== undefined
      && activity.blocked_after <= nowMs
      && nowMs < activity.blocked_until
    ) {
      state = "blocked";
      changedAt = activity.blocked_after;
    } else if (
      activity.ready_after !== undefined
      && activity.ready_until !== undefined
      && activity.open_tools.size === 0
      && activity.ready_after <= nowMs
      && nowMs < activity.ready_until
    ) {
      state = "ready";
      changedAt = activity.ready_after;
    } else if (activity.open_tools.size > 0 || activity.running_until > nowMs) {
      state = "running";
    }
    const recentlyCompleted = activity.completed_at !== undefined
      && nowMs < activity.completed_at + this.policy.recent_completion_ms;

    return {
      topic: activity.topic,
      state,
      state_changed_at: changedAt,
      last_event_at: activity.last_event_at,
      label: activity.label,
      provider: activity.session.provider,
      project_label: displayProjectName(activity.session.project),
      title: activity.session.title ?? undefined,
      session_id: activity.session.session_id,
      agent: activity.session.agent_nickname
        ?? activity.session.agent_path
        ?? undefined,
      completed_at: activity.completed_at,
      recently_completed: recentlyCompleted
    };
  }
}

function displayProjectName(project: RelaySession["project"]): string | undefined {
  return project?.project_name
    ?? project?.folder_name
    ?? project?.repository_name
    ?? project?.name
    ?? undefined;
}

function compareActiveSessions(left: PetFocus, right: PetFocus): number {
  const priority = STATE_PRIORITY[right.state] - STATE_PRIORITY[left.state];
  if (priority !== 0) {
    return priority;
  }
  const stateRecency = right.state_changed_at - left.state_changed_at;
  if (stateRecency !== 0) {
    return stateRecency;
  }
  const eventRecency = right.last_event_at - left.last_event_at;
  return eventRecency !== 0 ? eventRecency : compareTopics(left.topic, right.topic);
}

function compareRecentCompletions(left: PetFocus, right: PetFocus): number {
  const recency = (right.completed_at ?? 0) - (left.completed_at ?? 0);
  return recency !== 0 ? recency : compareTopics(left.topic, right.topic);
}

function compareTopics(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function newSessionActivity(relay: RelayEvent, nowMs: number): SessionActivity {
  return {
    topic: relay.topic,
    session: relay.session,
    last_event_at: nowMs,
    label: describeEvent(relay.event),
    running_until: nowMs,
    open_tools: new Map(),
    pending_interactions: new Map()
  };
}

export function describeEvent(event: AgentEvent): string {
  switch (event.type) {
    case "message": {
      const role = asString(event.role) ?? "message";
      const text = asString(event.text);
      return text ? `${role}: ${firstLine(text)}` : role;
    }
    case "reasoning":
      return "Thinking";
    case "tool_call":
      return describeTool(event);
    case "error":
      return asString(event.message) ?? "Blocked";
    case "goal_updated":
      return "Goal updated";
    case "agent_activity": {
      const kind = asString(event.kind) ?? "activity";
      const target = asString(event.target_agent_path);
      return target ? `${kind} ${target}` : kind;
    }
    case "unknown":
      return asString(event.native_type) ?? "Provider activity";
    case "session_started":
      return "Session started";
    case "provider_changed":
      return "Provider changed";
    case "session_settings_applied":
      return "Settings applied";
    default:
      return event.type.replaceAll("_", " ");
  }
}

function describeTool(event: AgentEvent): string {
  const summary = asObject(event.summary);
  const summaryType = asString(summary?.type);
  const path = asString(summary?.path);
  const command = asString(summary?.command);
  const query = asString(summary?.query);
  const url = asString(summary?.url);
  const title = asString(summary?.title);
  const detail = path ?? command ?? query ?? url ?? title;
  const name = asString(event.tool_name) ?? summaryType ?? "tool";
  return detail ? `${name}: ${firstLine(detail)}` : name;
}

function firstLine(value: string): string {
  return value.split(/\r?\n/, 1)[0]?.trim() ?? value;
}

function isInputTool(name: string): boolean {
  const normalized = name.split(".").at(-1)?.toLowerCase();
  return normalized === "request_user_input"
    || normalized === "ask_user_question"
    || normalized === "elicitation_request";
}
