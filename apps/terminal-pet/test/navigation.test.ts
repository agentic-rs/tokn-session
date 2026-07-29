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

  test("uses aggregate state and timing when a family root is selected", () => {
    const snapshot = petSnapshot(["root", "child"]);
    const root = snapshot.sessions[0]!;
    root.state = "idle";
    root.family_state = "needs_input";
    root.state_changed_at = 1;
    root.family_last_event_at = 42;
    const child = snapshot.sessions[1]!;
    child.parent_topic = root.topic;
    child.root_topic = root.topic;
    child.depth = 1;
    child.state = "needs_input";
    child.family_state = "needs_input";
    const focused = focusSnapshot(snapshot, root.topic);

    expect(focused.focus?.topic).toBe("root");
    expect(focused.state).toBe("needs_input");
    expect(focused.state_changed_at).toBe(42);
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
    root_topic: topic,
    depth: 0,
    is_provisional: false,
    state: topic === "two" ? "blocked" : "running",
    family_state: topic === "two" ? "blocked" : "running",
    family_last_event_at: index + 1,
    state_changed_at: index + 1,
    last_event_at: index + 1,
    label: topic,
    session_id: topic,
    recently_completed: false,
    descendant_count: 0,
    active_descendant_count: 0,
    urgent_descendant_count: 0,
    recent_descendant_count: 0
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
