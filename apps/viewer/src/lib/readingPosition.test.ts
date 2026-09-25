import { beforeEach, describe, expect, it, vi } from "vitest";
import { loadReadingWindow, readReadingPosition, saveReadingPosition, type ReadingPosition } from "./readingPosition";
import { RemoteClient, selectMachine } from "./transport";
import type { EventPageResponse, EventSummary } from "./types";

const position: ReadingPosition = {
  anchors: [{ slot_key: "event.v1.1", type: "message", timestamp: null, top: -50 }],
  last_event: "last", at_end: false,
};
const event = (slot_key: string) => ({ event_key: `window.new.${slot_key}`, slot_key, type: "message", timestamp: null } as EventSummary);
const page = (events: EventSummary[], previous_cursor: string | null = null): EventPageResponse => ({
  events, previous_cursor, next_cursor: null, total_events: events.length, history_status: "complete", attention_revision: "4",
});

beforeEach(() => { localStorage.clear(); selectMachine(); });

describe("reading positions", () => {
  it("persists positions separately by machine and session", () => {
    selectMachine(new RemoteClient("https://first.test", "secret"));
    saveReadingPosition("one", position);
    expect(readReadingPosition("one")).toEqual(position);
    expect(readReadingPosition("two")).toBeNull();
    selectMachine(new RemoteClient("https://second.test", "secret"));
    expect(readReadingPosition("one")).toBeNull();
    selectMachine(new RemoteClient("https://first.test", "secret"));
    expect(readReadingPosition("one")).toEqual(position);
    selectMachine();
  });

  it("ignores malformed persisted positions", () => {
    localStorage.setItem("tokn.viewer.reading-positions.v1", "null");
    expect(readReadingPosition("one")).toBeNull();
    localStorage.setItem("tokn.viewer.reading-positions.v1", "broken");
    saveReadingPosition("one", position);
    expect(readReadingPosition("one")).toEqual(position);
  });

  it("loads retained older turns until the saved anchor is available across generations", async () => {
    const load = vi.fn().mockResolvedValueOnce(page([event("event.v1.8")], "older"))
      .mockResolvedValueOnce(page([event("event.v1.1"), event("event.v1.8")], "oldest"));
    const response = await loadReadingWindow("one", position, load, () => true);
    expect(response.events[0].slot_key).toBe("event.v1.1");
    expect(load).toHaveBeenCalledTimes(2);
    expect(load).toHaveBeenLastCalledWith({ session_key: "one", window_mode: "earlier", direction: "backward", cursor: "older" });
  });

  it("stops restoring when selection changes or the server repeats a cursor", async () => {
    const load = vi.fn().mockResolvedValue(page([event("event.v1.8")], "older"));
    await loadReadingWindow("one", position, load, () => false);
    expect(load).toHaveBeenCalledTimes(1);
    load.mockClear();
    await loadReadingWindow("one", position, load, () => true);
    expect(load).toHaveBeenCalledTimes(2);
  });
});
