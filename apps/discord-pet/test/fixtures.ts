import type { RelayEvent } from "../src/protocol";

export function relayEvent(
  event: Record<string, unknown>,
  overrides: Partial<RelayEvent> = {}
): RelayEvent {
  return {
    topic: "codex.session-12345678",
    session: {
      provider: "codex",
      session_id: "session-12345678",
      parent_session_id: null,
      project: {
        project_name: "tokn-agent"
      }
    },
    event: {
      type: "message",
      ...event
    },
    ...overrides
  };
}
