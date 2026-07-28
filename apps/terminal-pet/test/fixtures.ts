import type { AgentEvent, RelayEvent } from "../src/protocol";

export function relayEvent(
  event: AgentEvent,
  topic = "codex.session-1"
): RelayEvent {
  return {
    path: "/tmp/session.jsonl",
    topic,
    session: {
      provider: topic.split(".", 1)[0],
      session_id: topic.split(".").slice(1).join("."),
      agent_path: "/root",
      project: {
        name: "tokn-agent",
        folder: "/tmp/tokn-agent"
      }
    },
    event
  };
}
