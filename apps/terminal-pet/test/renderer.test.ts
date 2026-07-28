import { describe, expect, test } from "bun:test";

import { loadPetArt } from "../src/art";
import { renderScreen } from "../src/renderer";
import type { PetSnapshot } from "../src/state";

describe("renderer", () => {
  test("renders a deterministic monochrome snapshot", async () => {
    const art = await loadPetArt();
    const snapshot: PetSnapshot = {
      state: "running",
      state_changed_at: 0,
      active_sessions: 1,
      total_sessions: 1,
      focus: {
        topic: "codex.session-1",
        state: "running",
        state_changed_at: 0,
        last_event_at: 0,
        label: "exec_command: cargo test",
        provider: "codex",
        project: "tokn-agent",
        session_id: "session-1"
      }
    };
    const screen = renderScreen(snapshot, art.running.ansi, {
      source_label: "test"
    }, {
      columns: 60,
      rows: 20,
      color: false,
      image_protocol: "ansi",
      name: "Hachiware"
    });
    const output = screen.lines.join("\n");

    expect(output).toContain("H A C H I W A R E");
    expect(output).toContain("● Running");
    expect(output).toContain("codex · tokn-agent");
    expect(output).toContain("exec_command: cargo test");
    expect(output).not.toContain("\u001b");
  });

  test("strips terminal controls and respects narrow display width", async () => {
    const art = await loadPetArt();
    const snapshot: PetSnapshot = {
      state: "running",
      state_changed_at: 0,
      active_sessions: 1,
      total_sessions: 1,
      focus: {
        topic: "codex.session-1",
        state: "running",
        state_changed_at: 0,
        last_event_at: 0,
        label: "\u001b]52;c;dGVzdA==\u0007 進捗進捗進捗進捗進捗進捗",
        provider: "codex",
        project: "tokn-agent",
        session_id: "session-1"
      }
    };
    const screen = renderScreen(snapshot, art.running.ansi, {
      source_label: "test"
    }, {
      columns: 30,
      rows: 20,
      color: false,
      image_protocol: "ansi",
      name: "Hachiware"
    });

    expect(screen.lines.join("\n")).not.toContain("\u001b");
    expect(screen.lines.every((line) => Bun.stringWidth(line) <= 30)).toBe(true);
  });
});
