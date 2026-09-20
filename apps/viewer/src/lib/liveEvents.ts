import type { EventPageResponse, EventSummary, LoadEventPageRequest, LoadTrajectoryEventPageRequest, TrajectoryEventPageResponse } from "./types";

/** The backend owns the retained turn window, including earlier loaded turns.
 * A refresh returns that whole window from one snapshot in a single request.
 */
export function refreshEventWindow(
  session_key: string,
  load: (request: LoadEventPageRequest) => Promise<EventPageResponse>,
): Promise<EventPageResponse> {
  return load({ session_key, window_mode: "retained" });
}

export async function refreshTrajectoryWindow(
  request: LoadTrajectoryEventPageRequest,
  previous: EventSummary[],
  load: (request: LoadTrajectoryEventPageRequest) => Promise<TrajectoryEventPageResponse>,
  current: () => boolean,
  reset = false,
): Promise<TrajectoryEventPageResponse> {
  let page = await load(request);
  const backward = request.direction === "backward";
  // Replacement generations may reuse source offsets for different events.
  // Preserve the window size across a reset, never its old event identities.
  const anchor = reset ? undefined
    : backward ? previous[0]?.event_key : previous[previous.length - 1]?.event_key;
  const cursors = new Set<string>();
  while (current() && previous.length > 0
    && (anchor ? !page.events.some((e) => e.event_key === anchor) : page.events.length < previous.length)) {
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
