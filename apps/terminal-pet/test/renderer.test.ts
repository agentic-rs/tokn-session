import { describe, expect, test } from "bun:test";

import { loadPetArt } from "../src/art";
import { renderScreen } from "../src/renderer";
import type { PetFocus, PetSnapshot } from "../src/state";

describe("renderer", () => {
  test("renders a deterministic monochrome session roster", async () => {
    const art = await loadPetArt();
    const running = petFocus({
      topic: "codex.session-1",
      label: "exec_command: cargo test"
    });
    const snapshot = petSnapshot([running]);
    const screen = renderScreen(snapshot, art.running.ansi, {
      source_label: "test"
    }, {
      columns: 100,
      rows: 24,
      color: false,
      image_protocol: "ansi",
      name: "Hachiware",
      now_ms: 0
    });
    const output = screen.lines.join("\n");

    expect(output).toContain("H A C H I W A R E");
    expect(output).toContain("SESSION ROSTER");
    expect(output).toContain("ACTIVE 1");
    expect(output).toContain("● Running");
    expect(output).toContain("tokn-agent/root");
    expect(output).toContain("exec_command: cargo test");
    expect(output).not.toContain("\u001b");
  });

  test("shows concurrent sessions and recently ready work", async () => {
    const art = await loadPetArt();
    const waiting = petFocus({
      topic: "codex.waiting",
      state: "needs_input",
      label: "Approval required",
      session_id: "waiting"
    });
    const running = petFocus({
      topic: "pi.running",
      label: "Running provider tests",
      provider: "pi",
      session_id: "running",
      agent: "worker"
    });
    const recent = petFocus({
      topic: "codex.recent",
      state: "idle",
      label: "Updated Relay docs",
      session_id: "recent",
      agent: "docs",
      completed_at: 30_000,
      recently_completed: true
    });
    const snapshot = petSnapshot([waiting, running, recent], 3);
    const screen = renderScreen(snapshot, art.waiting.ansi, {
      source_label: "test"
    }, {
      columns: 100,
      rows: 24,
      color: false,
      image_protocol: "ansi",
      name: "Hachiware",
      now_ms: 90_000
    });
    const output = screen.lines.join("\n");

    expect(output).toContain("ACTIVE 2");
    expect(output).toContain("RECENT READY 1");
    expect(output).toContain("? Needs input");
    expect(output).toContain("Running provider tests");
    expect(output).toContain("✓ Ready");
    expect(output).toContain("Updated Relay docs");
    expect(output).toContain("1 recent");
  });

  test("uses one image anchor alongside the wide roster", async () => {
    const art = await loadPetArt();
    const running = petFocus();
    const screen = renderScreen(petSnapshot([running]), art.running.ansi, {
      source_label: "test"
    }, {
      columns: 80,
      rows: 20,
      color: false,
      image_protocol: "kitty",
      name: "Hachiware",
      now_ms: 0
    });

    expect(screen.image_anchor).toEqual({
      column: 6,
      row: 3,
      columns: 10,
      rows: 5
    });
    expect(screen.lines.join("\n")).toContain("ACTIVE 1");
  });

  test("strips terminal controls and reports narrow-list overflow", async () => {
    const art = await loadPetArt();
    const sessions = Array.from({ length: 8 }, (_, index) => petFocus({
      topic: `codex.session-${index}`,
      session_id: `session-${index}`,
      label: index === 0
        ? "\u001b]52;c;dGVzdA==\u0007 進捗進捗進捗進捗進捗進捗"
        : `Working on task ${index}`,
      last_event_at: -index * 1_000
    }));
    const screen = renderScreen(petSnapshot(sessions, 8), art.running.ansi, {
      source_label: "test"
    }, {
      columns: 30,
      rows: 8,
      color: false,
      image_protocol: "kitty",
      name: "Hachiware",
      now_ms: 10_000
    });
    const output = screen.lines.join("\n");

    expect(screen.image_anchor).toBeUndefined();
    expect(output).not.toContain("\u001b");
    expect(output).toContain("more");
    expect(output).toContain("10s");
    expect(screen.lines.every((line) => Bun.stringWidth(line) <= 30)).toBe(true);
  });

  test("uses available rows on tall wide and narrow terminals", async () => {
    const art = await loadPetArt();
    const sessions = Array.from({ length: 20 }, (_, index) => petFocus({
      topic: `codex.session-${index}`,
      session_id: `session-${index}`,
      label: `Task ${index}`
    }));
    const wide = renderScreen(petSnapshot(sessions, 20), art.running.ansi, {
      source_label: "test"
    }, {
      columns: 80,
      rows: 40,
      color: false,
      image_protocol: "ansi",
      name: "Hachiware",
      now_ms: 0
    });
    const narrow = renderScreen(petSnapshot(sessions.slice(0, 6), 6), art.running.ansi, {
      source_label: "test"
    }, {
      columns: 23,
      rows: 12,
      color: false,
      image_protocol: "kitty",
      name: "Hachiware",
      now_ms: 0
    });

    expect(wide.lines.join("\n")).toContain("Task 19");
    expect(wide.lines.join("\n")).not.toContain("more");
    expect(narrow.image_anchor).toBeUndefined();
    expect(narrow.lines.join("\n")).toContain("ACTIVE 6");
    expect(narrow.lines.join("\n")).toContain("Task 5");
    expect(narrow.lines.every((line) => Bun.stringWidth(line) <= 23)).toBe(true);
  });

  test("prefers a session row over a heading in a two-row roster", async () => {
    const art = await loadPetArt();
    const sessions = Array.from({ length: 6 }, (_, index) => petFocus({
      topic: `codex.session-${index}`,
      session_id: `session-${index}`,
      label: `Task ${index}`
    }));
    const screen = renderScreen(petSnapshot(sessions, 6), art.running.ansi, {
      source_label: "test",
      diagnostic: "Relay warning"
    }, {
      columns: 24,
      rows: 6,
      color: false,
      image_protocol: "ansi",
      name: "Hachiware",
      now_ms: 0
    });
    const output = screen.lines.join("\n");

    expect(output).toContain("Task");
    expect(output).toContain("+5 more");
    expect(output).not.toContain("ACTIVE 6");
  });

  test("keeps diagnostics and provider identity in constrained layouts", async () => {
    const art = await loadPetArt();
    const running = petFocus({
      project: undefined,
      agent: undefined,
      title: undefined
    });
    const wide = renderScreen(petSnapshot([running]), art.running.ansi, {
      source_label: "test"
    }, {
      columns: 80,
      rows: 20,
      color: false,
      image_protocol: "ansi",
      name: "Hachiware",
      now_ms: 0
    });
    const tiny = renderScreen(petSnapshot([running]), art.running.ansi, {
      source_label: "test",
      diagnostic: "ERR"
    }, {
      columns: 7,
      rows: 4,
      color: false,
      image_protocol: "ansi",
      name: "Hachiware",
      now_ms: 0
    });

    expect(wide.lines.join("\n")).toContain("codex · session-");
    expect(tiny.lines.join("\n")).toContain("ERR");
    expect(tiny.lines.every((line) => Bun.stringWidth(line) <= 7)).toBe(true);
  });

  test("fills tight roster budgets before reporting overflow", async () => {
    const art = await loadPetArt();
    const active = petFocus({
      topic: "codex.active",
      session_id: "active",
      label: "Active task"
    });
    const recent = petFocus({
      topic: "codex.recent",
      state: "idle",
      session_id: "recent",
      label: "Recent task",
      completed_at: 0,
      recently_completed: true
    });
    const twoSessions = renderScreen(petSnapshot([active, recent], 2), art.running.ansi, {
      source_label: "test"
    }, {
      columns: 24,
      rows: 6,
      color: false,
      image_protocol: "ansi",
      name: "Hachiware",
      now_ms: 10_000
    });
    const fourSessions = Array.from({ length: 4 }, (_, index) => petFocus({
      topic: `codex.session-${index}`,
      session_id: `session-${index}`,
      label: `Task ${index}`
    }));
    const fiveRows = renderScreen(petSnapshot(fourSessions, 4), art.running.ansi, {
      source_label: "test"
    }, {
      columns: 40,
      rows: 5,
      color: false,
      image_protocol: "ansi",
      name: "Hachiware",
      now_ms: 0
    });

    expect(twoSessions.lines.join("\n")).toContain("Active");
    expect(twoSessions.lines.join("\n")).toContain("Recent");
    expect(twoSessions.lines.join("\n")).not.toContain("more");
    expect(fiveRows.lines.join("\n")).toContain("Task 0");
    expect(fiveRows.lines.join("\n")).toContain("+3 more");
  });

  test("preserves activity age across intermediate narrow widths", async () => {
    const art = await loadPetArt();
    const running = petFocus();

    for (let columns = 8; columns < 32; columns += 1) {
      const screen = renderScreen(petSnapshot([running]), art.running.ansi, {
        source_label: "test"
      }, {
        columns,
        rows: 8,
        color: false,
        image_protocol: "ansi",
        name: "Hachiware",
        now_ms: 10_000
      });
      expect(screen.lines.join("\n")).toContain("10s");
      expect(screen.lines.every((line) => Bun.stringWidth(line) <= columns)).toBe(true);
    }
  });
});

function petFocus(overrides: Partial<PetFocus> = {}): PetFocus {
  return {
    topic: "codex.session-1",
    state: "running",
    state_changed_at: 0,
    last_event_at: 0,
    label: "Thinking",
    provider: "codex",
    project: "tokn-agent",
    session_id: "session-1",
    agent: "root",
    recently_completed: false,
    ...overrides
  };
}

function petSnapshot(sessions: PetFocus[], totalSessions = sessions.length): PetSnapshot {
  const focus = sessions[0];
  return {
    state: focus?.state ?? "idle",
    state_changed_at: focus?.state_changed_at ?? 0,
    active_sessions: sessions.filter((session) => session.state !== "idle").length,
    total_sessions: totalSessions,
    sessions,
    focus
  };
}
