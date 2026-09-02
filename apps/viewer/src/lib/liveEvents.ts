import type { EventPageResponse, EventSummary, LoadEventPageRequest, LoadTrajectoryEventPageRequest, TrajectoryEventPageResponse } from "./types";

/** Refresh the loaded window from one pinned snapshot using bounded pages.
 * Keep earlier loaded rows instead of replacing the page with the newest 100.
 * Calls are serialized by the owner; stale selections stop between requests.
 */
export async function refreshEventWindow(
  session_key: string,
  previous: EventSummary[],
  page_size: number,
  load: (request: LoadEventPageRequest) => Promise<EventPageResponse>,
  current: () => boolean,
  reset: boolean,
): Promise<EventPageResponse> {
  let page = await load({ session_key, direction: "backward", limit: page_size });
  const first_key = reset ? undefined : previous[0]?.event_key;
  const cursors = new Set<string>();
  while (current() && page.previous_cursor && previous.length > 0
    && (first_key ? !page.events.some((e) => e.event_key === first_key) : page.events.length < previous.length)) {
    const cursor = page.previous_cursor;
    if (cursors.has(cursor)) throw new Error("Session refresh returned a repeated cursor");
    cursors.add(cursor);
    const older = await load({ session_key, cursor, direction: "backward", limit: page_size });
    page = { ...page, previous_cursor: older.previous_cursor, events: [...older.events, ...page.events] };
  }
  // Keep the complete returned page: its previous_cursor belongs to its start.
  // Cropping would manufacture a cursor and could skip history on the next load.
  return page;
}

export async function refreshTrajectoryWindow(
  request: LoadTrajectoryEventPageRequest,
  previous: EventSummary[],
  load: (request: LoadTrajectoryEventPageRequest) => Promise<TrajectoryEventPageResponse>,
  current: () => boolean,
): Promise<TrajectoryEventPageResponse> {
  let page = await load(request);
  const backward = request.direction === "backward";
  const anchor = backward ? previous[0]?.event_key : previous[previous.length - 1]?.event_key;
  const cursors = new Set<string>();
  while (current() && anchor && !page.events.some((e) => e.event_key === anchor)) {
    const cursor = backward ? page.previous_cursor : page.next_cursor;
    if (!cursor) break;
    if (cursors.has(cursor)) throw new Error("Turn refresh returned a repeated cursor");
    cursors.add(cursor);
    const more = await load({ ...request, cursor });
    page = backward
      ? { ...page, previous_cursor: more.previous_cursor, events: [...more.events, ...page.events] }
      : { ...page, next_cursor: more.next_cursor, events: [...page.events, ...more.events] };
  }
  return page;
}
