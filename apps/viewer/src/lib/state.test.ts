import { describe, expect, it } from "vitest";
import type { SessionSummary } from "./types";
import { sessionDisplayTitle, shortSessionId } from "./state";

function session(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    session_key: "key",
    session_id: "01991dce-7f6a-7000-8000-000000000001",
    parent_session_id: null,
    provider: "codex",
    title: null,
    preview: null,
    project: null,
    cwd: null,
    updated_at_ms: null,
    timestamp: null,
    agent_path: null,
    message_count: null,
    event_count: null,
    history_status: null,
    ...overrides,
  };
}

describe("session identity", () => {
  it("prefers a native title over the first-message preview", () => {
    expect(sessionDisplayTitle(session({ title: "Named task", preview: "Raw prompt" })))
      .toBe("Named task");
  });

  it("uses a meaningful preview and then a neutral fallback", () => {
    expect(sessionDisplayTitle(session({ preview: "  Build the viewer  " })))
      .toBe("Build the viewer");
    expect(sessionDisplayTitle(session({ title: "  ", preview: "\n\t" })))
      .toBe("Untitled session");
  });

  it("keeps short ids intact and abbreviates long ids", () => {
    expect(shortSessionId("session-12")).toBe("session-12");
    expect(shortSessionId("01991dce-7f6a-7000-8000-000000000001"))
      .toBe("01991dce…");
  });
});
