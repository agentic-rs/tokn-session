import type { RelayEvent } from "@tokn/discord-pet/protocol";

export function relayEvent(
  overrides: Partial<RelayEvent> = {}
): RelayEvent {
  return {
    topic: "codex.session",
    session: {
      provider: "codex",
      session_id: "session",
      parent_session_id: null,
      project: {
        repository_name: "volty-web"
      }
    },
    event: {
      type: "message",
      role: "user",
      delivery: "unspecified",
      phase: "finished",
      text: "hello"
    },
    ...overrides
  };
}
