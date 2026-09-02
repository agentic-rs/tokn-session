import { describe, expect, it, vi } from "vitest";
import { refreshEventWindow, refreshTrajectoryWindow } from "./liveEvents";
import type { EventPageResponse, EventSummary } from "./types";

const event = (event_key: string, summary = event_key) => ({ event_key, summary }) as EventSummary;
const page = (keys: string[], previous_cursor: string | null): EventPageResponse => ({
  events: keys.map((key) => event(key, `updated ${key}`)), previous_cursor, next_cursor: null, total_events: 6, history_status: "complete",
});

describe("live event windows", () => {
  it("refreshes earlier loaded rows as well as the latest page", async () => {
    const load = vi.fn().mockResolvedValueOnce(page(["e", "f"], "older-4"))
      .mockResolvedValueOnce(page(["c", "d"], "older-2"))
      .mockResolvedValueOnce(page(["a", "b"], null));
    const result = await refreshEventWindow("session", [event("b"), event("c"), event("d")], 2, load, () => true, false);
    expect(result.events.map((e) => e.summary)).toEqual(["a", "b", "c", "d", "e", "f"].map((key) => `updated ${key}`));
    expect(result.previous_cursor).toBeNull();
    expect(load).toHaveBeenNthCalledWith(2, { session_key: "session", cursor: "older-4", direction: "backward", limit: 2 });
  });

  it("stops paging an obsolete selection", async () => {
    const load = vi.fn().mockResolvedValue(page(["e", "f"], "older"));
    await refreshEventWindow("session", [event("a")], 2, load, () => false, false);
    expect(load).toHaveBeenCalledOnce();
  });

  it("refreshes the loaded child window starting with the latest active work", async () => {
    const load = vi.fn().mockResolvedValueOnce(page(["c", "d"], "older"))
      .mockResolvedValueOnce(page(["a", "b"], null));
    const result = await refreshTrajectoryWindow({ session_key: "s", trajectory_key: "t", direction: "backward", limit: 2 }, [event("b")], load, () => true);
    expect(result.events.map((e) => e.event_key)).toEqual(["a", "b", "c", "d"]);
  });
});
