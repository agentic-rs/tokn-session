import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { describe, expect, test } from "bun:test";

import {
  CodexInputBroker,
  codexCliResumeCommand,
  codexInputAdmissionStatus,
  readRolloutSettings,
  type CodexSessionTarget,
} from "../src/codex_input";
import { relayEvent } from "./fixtures";

describe("CodexInputBroker", () => {
  test("submits an observed root session through desktop IPC", async () => {
    await withRollout(async (path) => {
      const requests: string[] = [];
      const broker = new CodexInputBroker({
        ipc_submit: async (target, prompt) => {
          requests.push(`${target.session_id}:${prompt}`);
          return {
            conversation_id: target.session_id,
            request_id: "ipc-request",
            handled_by_client_id: "desktop-owner",
            result: {}
          };
        },
        cli_resume: async () => {
          throw new Error("CLI should not run");
        }
      });
      const event = codexEvent(path);
      broker.observe(event);

      const admission = await broker.submit(event.topic, "  continue  ");

      expect(requests).toEqual(["session-1:continue"]);
      expect(admission.route).toBe("ipc");
      expect(codexInputAdmissionStatus(admission)).toBe("Codex App accepted input");
    });
  });

  test("falls back to CLI when no desktop owner exists", async () => {
    await withRollout(async (path) => {
      const cliPrompts: string[] = [];
      const broker = new CodexInputBroker({
        ipc_submit: async () => {
          throw new Error("Codex desktop IPC thread-follower-start-turn failed: no-client-found");
        },
        cli_resume: async (_target, prompt) => {
          cliPrompts.push(prompt);
        },
        read_rollout_settings: async () => ({})
      });
      const event = codexEvent(path);
      broker.observe(event);

      const admission = await broker.submit(event.topic, "continue");

      expect(cliPrompts).toEqual(["continue"]);
      expect(admission.route).toBe("cli");
      expect(codexInputAdmissionStatus(admission)).toBe("Codex CLI completed input");
    });
  });

  test("refuses CLI fallback without the observed session cwd", async () => {
    await withRollout(async (path) => {
      let cliCalls = 0;
      const broker = new CodexInputBroker({
        ipc_submit: async () => {
          throw new Error("Codex desktop IPC thread-follower-start-turn failed: no-client-found");
        },
        cli_resume: async () => {
          cliCalls += 1;
        },
        read_rollout_settings: async () => ({})
      });
      const event = codexEvent(path);
      event.session.cwd = null;
      broker.observe(event);

      await expect(broker.submit(event.topic, "continue")).rejects.toThrow(
        "observed absolute session cwd"
      );
      expect(cliCalls).toBe(0);
    });
  });

  test("does not retry ambiguous IPC failures through CLI", async () => {
    await withRollout(async (path) => {
      let cliCalls = 0;
      const broker = new CodexInputBroker({
        ipc_submit: async () => {
          throw new Error("Codex desktop IPC request timed out");
        },
        cli_resume: async () => {
          cliCalls += 1;
        },
        read_rollout_settings: async () => ({})
      });
      const event = codexEvent(path);
      broker.observe(event);

      await expect(broker.submit(event.topic, "continue")).rejects.toThrow("timed out");
      expect(cliCalls).toBe(0);
    });
  });

  test("does not treat owner rejection text as a missing owner", async () => {
    await withRollout(async (path) => {
      let cliCalls = 0;
      const broker = new CodexInputBroker({
        ipc_submit: async () => {
          throw new Error("owner rejected prompt containing no-client-found");
        },
        cli_resume: async () => {
          cliCalls += 1;
        },
        read_rollout_settings: async () => ({})
      });
      const event = codexEvent(path);
      broker.observe(event);

      await expect(broker.submit(event.topic, "continue")).rejects.toThrow(
        "owner rejected"
      );
      expect(cliCalls).toBe(0);
    });
  });

  test("rejects subagent sessions", async () => {
    await withRollout(async (path) => {
      const broker = new CodexInputBroker({
        ipc_submit: async () => {
          throw new Error("should not submit");
        }
      });
      const event = codexEvent(path);
      event.session.parent_session_id = "parent-session";
      event.session.agent_path = "/root/researcher";
      broker.observe(event);

      await expect(broker.submit(event.topic, "continue")).rejects.toThrow("root sessions");
    });
  });

  test("allows only one submission per rollout at a time", async () => {
    await withRollout(async (path) => {
      let release!: () => void;
      const pending = new Promise<void>((resolve) => {
        release = resolve;
      });
      const broker = new CodexInputBroker({
        strategy: "cli",
        cli_resume: async () => pending,
        read_rollout_settings: async () => ({})
      });
      const event = codexEvent(path);
      broker.observe(event);

      const first = broker.submit(event.topic, "first");
      await expect(broker.submit(event.topic, "second")).rejects.toThrow("input in flight");
      release();
      await first;
    });
  });

  test("retains rollout model settings for CLI and applies explicit overrides", async () => {
    await withRollout(async (path) => {
      const received: unknown[] = [];
      const broker = new CodexInputBroker({
        strategy: "cli",
        cli_resume: async (_target, _prompt, overrides) => {
          received.push(overrides);
        },
        read_rollout_settings: async () => ({
          model: "gpt-5.6-sol",
          effort: "xhigh"
        })
      });
      const event = codexEvent(path);
      broker.observe(event);

      await broker.submit(event.topic, "continue", { effort: "low" });

      expect(received).toEqual([{ model: "gpt-5.6-sol", effort: "low" }]);
    });
  });

  test("forwards optional model and effort to the selected route", async () => {
    await withRollout(async (path) => {
      const received: unknown[] = [];
      const broker = new CodexInputBroker({
        strategy: "ipc",
        ipc_submit: async (target, _prompt, overrides) => {
          received.push(overrides);
          return {
            conversation_id: target.session_id,
            request_id: "ipc-request",
            handled_by_client_id: "desktop-owner",
            result: {}
          };
        }
      });
      const event = codexEvent(path);
      broker.observe(event);

      await broker.submit(event.topic, "continue", {
        model: "gpt-5.6-luna",
        effort: "low"
      });

      expect(received).toEqual([{ model: "gpt-5.6-luna", effort: "low" }]);
    });
  });
});

describe("readRolloutSettings", () => {
  test("uses the latest persisted Codex model and effort", async () => {
    await withRollout(async (path) => {
      writeFileSync(path, [
        JSON.stringify({
          type: "event_msg",
          payload: {
            type: "thread_settings_applied",
            thread_settings: { model: "gpt-5.6-sol", reasoning_effort: "high" }
          }
        }),
        "not-json",
        JSON.stringify({
          type: "event_msg",
          payload: {
            type: "thread_settings_applied",
            thread_settings: { model: "gpt-5.6-luna", reasoning_effort: "low" }
          }
        }),
        JSON.stringify({
          type: "turn_context",
          payload: { model: "gpt-5.6-terra", effort: "medium" }
        })
      ].join("\n"));

      await expect(readRolloutSettings(path)).resolves.toEqual({
        model: "gpt-5.6-terra",
        effort: "medium"
      });
    });
  });
});

describe("codexCliResumeCommand", () => {
  test("uses stdin and explicit model settings without a shell", () => {
    const target: CodexSessionTarget = {
      provider: "codex",
      path: "/sessions/rollout.jsonl",
      session_id: "session-1",
      cwd: "/workspace"
    };

    expect(codexCliResumeCommand("codex", target, {
      model: "gpt-5.6-luna",
      effort: "low"
    })).toEqual([
      "codex",
      "exec",
      "resume",
      "--json",
      "--skip-git-repo-check",
      "--model",
      "gpt-5.6-luna",
      "-c",
      "model_reasoning_effort=\"low\"",
      "session-1",
      "-"
    ]);
  });
});

function codexEvent(path: string) {
  const event = relayEvent({ type: "message", role: "assistant" }, "codex.session-1");
  event.path = path;
  event.session.provider = "codex";
  event.session.session_id = "session-1";
  event.session.cwd = dirname(path);
  return event;
}

async function withRollout(run: (path: string) => Promise<void>): Promise<void> {
  const directory = mkdtempSync(join(tmpdir(), "terminal-pet-codex-input-"));
  try {
    const path = join(directory, "rollout.jsonl");
    writeFileSync(path, "{}\n");
    await run(path);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}
