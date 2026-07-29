import { describe, expect, test } from "bun:test";

import { focusSnapshot, moveFocusTopic } from "../src/navigation";
import type { PetFocus, PetSnapshot } from "../src/state";

describe("session focus navigation", () => {
  test("wraps through the sorted roster from automatic focus", () => {
    const snapshot = petSnapshot(["one", "two", "three"]);

    expect(moveFocusTopic(snapshot, undefined, "next")).toBe("two");
    expect(moveFocusTopic(snapshot, undefined, "previous")).toBe("three");
    expect(moveFocusTopic(snapshot, "three", "next")).toBe("one");
    expect(moveFocusTopic(snapshot, "one", "previous")).toBe("three");
  });

  test("keeps selection stable by topic when the roster reorders", () => {
    const original = petSnapshot(["one", "two", "three"]);
    const reordered = petSnapshot(["three", "one", "two"]);
    const selected = focusSnapshot(original, "two");
    const stillSelected = focusSnapshot(reordered, selected.focus?.topic);

    expect(stillSelected.focus?.topic).toBe("two");
    expect(stillSelected.state).toBe("blocked");
    expect(stillSelected.state_changed_at).toBe(3);
  });

  test("falls back to automatic focus when a topic disappears", () => {
    const snapshot = petSnapshot(["one", "two"]);
    const focused = focusSnapshot(snapshot, "missing");

    expect(focused).toBe(snapshot);
    expect(focused.focus?.topic).toBe("one");
    expect(moveFocusTopic(snapshot, "missing", "next")).toBe("one");
    expect(moveFocusTopic(snapshot, "missing", "previous")).toBe("two");
  });

  test("stays automatic when a single-session roster cannot move", () => {
    const snapshot = petSnapshot(["one"]);

    expect(moveFocusTopic(snapshot, undefined, "next")).toBeUndefined();
    expect(moveFocusTopic(snapshot, undefined, "previous")).toBeUndefined();
  });
});

function petSnapshot(topics: string[]): PetSnapshot {
  const sessions = topics.map((topic, index): PetFocus => ({
    topic,
    state: topic === "two" ? "blocked" : "running",
    state_changed_at: index + 1,
    last_event_at: index + 1,
    label: topic,
    session_id: topic,
    recently_completed: false
  }));
  const focus = sessions[0];
  return {
    state: focus?.state ?? "idle",
    state_changed_at: focus?.state_changed_at ?? 0,
    active_sessions: sessions.length,
    total_sessions: sessions.length,
    sessions,
    focus
  };
}
