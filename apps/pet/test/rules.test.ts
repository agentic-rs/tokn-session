import { describe, expect, test } from "bun:test";

import { RuleEngine } from "../src/rules";
import { relayEvent } from "./fixtures";

describe("RuleEngine", () => {
  const engine = new RuleEngine([
    {
      forward_to: ["terminal"]
    },
    {
      forward_to: ["discord_volty"],
      when: {
        root_only: true,
        repository_names: ["volty*"],
        event_types: ["message"],
        roles: ["user"]
      }
    },
    {
      forward_to: ["discord_volty"],
      when: {
        root_only: true,
        repository_names: ["volty*"],
        event_types: ["message"],
        roles: ["assistant"],
        deliveries: ["final"]
      }
    }
  ]);

  test("forwards every event to terminal", () => {
    expect(engine.targets(relayEvent({
      event: {
        type: "tool_call",
        phase: "started"
      }
    }))).toEqual(["terminal"]);
  });

  test("matches volty repositories case-insensitively", () => {
    expect(engine.targets(relayEvent())).toEqual([
      "terminal",
      "discord_volty"
    ]);
    expect(engine.targets(relayEvent({
      session: {
        provider: "codex",
        session_id: "session",
        parent_session_id: null,
        project: {
          repository_name: "VoltyDesktop"
        }
      }
    }))).toEqual([
      "terminal",
      "discord_volty"
    ]);
  });

  test("keeps non-volty and child messages out of Discord", () => {
    expect(engine.targets(relayEvent({
      session: {
        provider: "codex",
        session_id: "session",
        parent_session_id: null,
        project: {
          repository_name: "tokn-session"
        }
      }
    }))).toEqual(["terminal"]);
    expect(engine.targets(relayEvent({
      session: {
        provider: "codex",
        session_id: "child",
        parent_session_id: "session",
        project: {
          repository_name: "volty-web"
        }
      }
    }))).toEqual(["terminal"]);
  });

  test("forwards only final assistant messages to Discord", () => {
    expect(engine.targets(relayEvent({
      event: {
        type: "message",
        role: "assistant",
        delivery: "commentary",
        phase: "finished",
        text: "working"
      }
    }))).toEqual(["terminal"]);
    expect(engine.targets(relayEvent({
      event: {
        type: "message",
        role: "assistant",
        delivery: "final",
        phase: "finished",
        text: "done"
      }
    }))).toEqual([
      "terminal",
      "discord_volty"
    ]);
  });

  test("deduplicates a worker matched by several rules", () => {
    const duplicateEngine = new RuleEngine([
      { forward_to: ["terminal"] },
      { forward_to: ["terminal"] }
    ]);

    expect(duplicateEngine.targets(relayEvent())).toEqual(["terminal"]);
  });
});
