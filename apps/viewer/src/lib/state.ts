import type { EventDetail, EventSummary, JsonValue, SessionSummary, ViewerProvider } from "./types";

export const SESSION_PAGE_SIZE = 60;
export const EVENT_PAGE_SIZE = 80;

export interface ReadableContentSection {
  label: string | null;
  text: string;
}

export interface ReadableEventContent {
  sections: ReadableContentSection[];
}

function jsonObject(value: JsonValue): { [key: string]: JsonValue } | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}

function readableString(value: JsonValue | undefined): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

export function readableEventContent(
  summary: EventSummary,
  detail: EventDetail | null,
): ReadableEventContent | null {
  if (!detail || detail.is_hidden || summary.is_hidden) {
    return null;
  }
  const event = jsonObject(detail.event);
  if (!event || event.type !== summary.type) {
    return null;
  }
  if (summary.type === "message") {
    const text = readableString(event.text);
    return text ? { sections: [{ label: null, text }] } : null;
  }
  if (summary.type !== "reasoning") {
    return null;
  }

  const reasoningSummary = readableString(event.summary);
  const reasoningText = readableString(event.text);
  if (!reasoningSummary && !reasoningText) {
    return null;
  }
  if (reasoningSummary && reasoningText && reasoningSummary !== reasoningText) {
    return {
      sections: [
        { label: "Summary", text: reasoningSummary },
        { label: "Reasoning", text: reasoningText },
      ],
    };
  }
  return {
    sections: [{
      label: reasoningSummary ? "Summary" : "Reasoning",
      text: reasoningSummary ?? reasoningText!,
    }],
  };
}

export function eventButtonId(eventKey: string): string {
  return `event-button-${eventKey.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
}

export function errorMessage(error: unknown): string {
  if (typeof error === "string") {
    return error;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return "An unexpected error occurred.";
}

export function mergeSessions(
  current: SessionSummary[],
  incoming: SessionSummary[],
): SessionSummary[] {
  const byKey = new Map(current.map((session) => [session.session_key, session]));
  for (const session of incoming) {
    byKey.set(session.session_key, session);
  }
  return [...byKey.values()].sort((left, right) => {
    return (right.updated_at_ms ?? 0) - (left.updated_at_ms ?? 0);
  });
}

export function mergeEvents(
  current: EventSummary[],
  incoming: EventSummary[],
  position: "before" | "after",
): EventSummary[] {
  const ordered = position === "before" ? [...incoming, ...current] : [...current, ...incoming];
  const seen = new Set<string>();
  return ordered.filter((event) => {
    if (seen.has(event.event_key)) {
      return false;
    }
    seen.add(event.event_key);
    return true;
  });
}

export function preserveSessionSelection(
  currentKey: string | null,
  sessions: SessionSummary[],
): string | null {
  if (currentKey && sessions.some((session) => session.session_key === currentKey)) {
    return currentKey;
  }
  return sessions[0]?.session_key ?? null;
}

export function preserveEventSelection(
  currentKey: string | null,
  events: EventSummary[],
): string | null {
  if (currentKey && events.some((event) => event.event_key === currentKey)) {
    return currentKey;
  }
  return null;
}

export function groupSessions(sessions: SessionSummary[]): Array<{
  key: string;
  project: string;
  sessions: SessionSummary[];
}> {
  const groups = new Map<string, { project: string; sessions: SessionSummary[] }>();
  for (const session of sessions) {
    const project = session.project?.trim() || session.cwd?.trim() || "Other sessions";
    const key = session.cwd?.trim() || project;
    const group = groups.get(key) ?? { project, sessions: [] };
    group.sessions.push(session);
    groups.set(key, group);
  }
  return [...groups].map(([key, group]) => ({ key, ...group }));
}

export function providerLabel(provider: ViewerProvider): string {
  switch (provider) {
    case "codex":
      return "Codex";
    case "pi":
      return "Pi";
    case "opencode":
      return "OpenCode";
    case "dsh":
      return "DSH";
  }
}

function timestampMs(timestamp: string | null): number {
  if (!timestamp) {
    return Number.NaN;
  }
  const numeric = Number(timestamp);
  if (Number.isFinite(numeric)) {
    return numeric < 100_000_000_000 ? numeric * 1_000 : numeric;
  }
  return Date.parse(timestamp);
}

export function formatRelativeTime(timestamp: string | null, epochMs: number | null): string {
  const parsed = epochMs ?? timestampMs(timestamp);
  if (!Number.isFinite(parsed)) {
    return "Unknown time";
  }

  const elapsed = Date.now() - parsed;
  const absoluteElapsed = Math.abs(elapsed);
  const suffix = elapsed >= 0 ? "ago" : "from now";
  if (absoluteElapsed < 60_000) {
    return "Just now";
  }
  if (absoluteElapsed < 3_600_000) {
    return `${Math.floor(absoluteElapsed / 60_000)}m ${suffix}`;
  }
  if (absoluteElapsed < 86_400_000) {
    return `${Math.floor(absoluteElapsed / 3_600_000)}h ${suffix}`;
  }
  if (absoluteElapsed < 604_800_000) {
    return `${Math.floor(absoluteElapsed / 86_400_000)}d ${suffix}`;
  }
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(parsed);
}

export function formatTimestamp(timestamp: string | null): string {
  if (!timestamp) {
    return "Timestamp unavailable";
  }
  const parsed = timestampMs(timestamp);
  if (!Number.isFinite(parsed)) {
    return timestamp;
  }
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(parsed);
}
