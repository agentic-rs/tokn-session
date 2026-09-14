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

  it.each([
    { name: "reuses an old ordinal key", latest: ["a", "h"] },
    { name: "replaces every event key", latest: ["g", "h"] },
  ])("preserves the loaded child count when a reset $name", async ({ latest }) => {
    const load = vi.fn().mockResolvedValueOnce(page(latest, "older-6"))
      .mockResolvedValueOnce(page(["e", "f"], "older-4"));
    const result = await refreshTrajectoryWindow(
      { session_key: "s", trajectory_key: "t", direction: "backward", limit: 2 },
      [event("a"), event("b"), event("c")], load, () => true, true,
    );
    expect(result.events.map((e) => e.event_key)).toEqual(["e", "f", ...latest]);
    expect(result.previous_cursor).toBe("older-4");
    expect(load).toHaveBeenCalledTimes(2);
    expect(load).toHaveBeenLastCalledWith({ session_key: "s", trajectory_key: "t", cursor: "older-6", direction: "backward", limit: 2 });
  });

  it("stops a reset child refresh when the replacement has fewer rows", async () => {
    const load = vi.fn().mockResolvedValueOnce(page(["y", "z"], "older"))
      .mockResolvedValueOnce(page(["x"], null));
    const result = await refreshTrajectoryWindow(
      { session_key: "s", trajectory_key: "t", direction: "backward", limit: 2 },
      [event("a"), event("b"), event("c"), event("d")], load, () => true, true,
    );
    expect(result.events.map((e) => e.event_key)).toEqual(["x", "y", "z"]);
    expect(result.previous_cursor).toBeNull();
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("rejects a repeated cursor during a reset child refresh", async () => {
    const load = vi.fn().mockResolvedValueOnce(page(["z"], "older"))
      .mockResolvedValueOnce(page(["y"], "older"));
    await expect(refreshTrajectoryWindow(
      { session_key: "s", trajectory_key: "t", direction: "backward", limit: 1 },
      [event("a"), event("b"), event("c")], load, () => true, true,
    )).rejects.toThrow("Turn refresh returned a repeated cursor");
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("stops paging reset children when the request becomes obsolete", async () => {
    const load = vi.fn().mockResolvedValueOnce(page(["z"], "older-2"))
      .mockResolvedValueOnce(page(["y"], "older-1"));
    const current = vi.fn().mockReturnValueOnce(true).mockReturnValue(false);
    const result = await refreshTrajectoryWindow(
      { session_key: "s", trajectory_key: "t", direction: "backward", limit: 1 },
      [event("a"), event("b"), event("c")], load, current, true,
    );
    expect(result.events.map((e) => e.event_key)).toEqual(["y", "z"]);
    expect(result.previous_cursor).toBe("older-1");
    expect(load).toHaveBeenCalledTimes(2);
  });
});
