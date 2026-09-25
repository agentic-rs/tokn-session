import { viewerStorageScope } from "./transport";
import type { EventPageResponse, EventSummary, LoadEventPageRequest } from "./types";

export interface ReadingAnchor {
  slot_key: string;
  type: string;
  timestamp: string | null;
  top: number;
}

export interface ReadingPosition {
  anchors: ReadingAnchor[];
  last_event: string;
  at_end: boolean;
}

const STORAGE_KEY = "tokn.viewer.reading-positions.v1";
const MAX_POSITIONS = 200;

function positionKey(session_key: string) {
  return JSON.stringify([viewerStorageScope(), session_key]);
}

function positions(): Record<string, ReadingPosition> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}");
    return value && typeof value === "object" && !Array.isArray(value)
      ? value as Record<string, ReadingPosition> : {};
  }
  catch { return {}; }
}

export function readReadingPosition(session_key: string): ReadingPosition | null {
  try {
    const value = positions()[positionKey(session_key)];
    if (!value || typeof value.last_event !== "string" || typeof value.at_end !== "boolean"
      || !Array.isArray(value.anchors) || !value.anchors.length
      || !value.anchors.every((anchor) => typeof anchor.slot_key === "string"
        && typeof anchor.type === "string" && (anchor.timestamp === null || typeof anchor.timestamp === "string")
        && Number.isFinite(anchor.top))) return null;
    return value;
  } catch { return null; }
}

export function saveReadingPosition(session_key: string, position: ReadingPosition) {
  try {
    const all = positions();
    const key = positionKey(session_key);
    delete all[key];
    all[key] = position;
    localStorage.setItem(STORAGE_KEY, JSON.stringify(Object.fromEntries(Object.entries(all).slice(-MAX_POSITIONS))));
  } catch { /* Reading remains usable with disabled or full storage. */ }
}

export function readingEventKey(event: EventSummary): string {
  return JSON.stringify([event.slot_key ?? event.event_key, event.type, event.timestamp]);
}

export function matchesReadingAnchor(event: EventSummary, anchor: ReadingAnchor): boolean {
  return (event.slot_key ?? event.event_key) === anchor.slot_key
    && event.type === anchor.type && event.timestamp === anchor.timestamp;
}

/** Load older retained turns before publishing a restored position to the UI. */
export async function loadReadingWindow(
  session_key: string,
  position: ReadingPosition | null,
  load: (request: LoadEventPageRequest) => Promise<EventPageResponse>,
  is_current: () => boolean,
): Promise<EventPageResponse> {
  let page = await load({ session_key, window_mode: "retained", direction: "backward" });
  let fallback = page;
  const cursors = new Set<string>();
  while (position && is_current() && page.previous_cursor
    && !page.events.some((event) => matchesReadingAnchor(event, position.anchors[0]))) {
    const cursor = page.previous_cursor;
    if (cursors.has(cursor)) break;
    cursors.add(cursor);
    page = await load({ session_key, window_mode: "earlier", direction: "backward", cursor });
    if (position.anchors.some((anchor) => page.events.some((event) => matchesReadingAnchor(event, anchor)))) {
      fallback = page;
    }
  }
  if (position && !page.events.some((event) => matchesReadingAnchor(event, position.anchors[0]))) {
    // A rollback, changed projection, or removed row can invalidate a bookmark.
    // Never publish the whole backfilled history merely because the search
    // exhausted it: that turns a missing anchor into a jump to session start.
    return fallback;
  }
  return page;
}
