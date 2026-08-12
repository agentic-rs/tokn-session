import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { CodexDesktopInputClient } from "../lib/desktop-input-client";
import {
  CODEX_WINDOWS_PIPE_NAME,
  codexDesktopIpcEndpoint,
  type CodexIpcEndpoint,
} from "../lib/ipc-endpoint";
import { encodeIpcFrame, IpcFrameDecoder } from "../lib/ipc-protocol";
import { FakeCodexDesktopOwner, FakeCodexDesktopRouter } from "../lab/fake-desktop";

describe("Codex desktop IPC framing", () => {
  test("decodes split and coalesced frames", () => {
    const first = encodeIpcFrame({ sequence: 1 });
    const second = encodeIpcFrame({ sequence: 2 });
    const decoder = new IpcFrameDecoder();

    expect(decoder.push(first.subarray(0, 3))).toEqual([]);
    expect(decoder.push(Buffer.concat([first.subarray(3), second]))).toEqual([
      { sequence: 1 },
      { sequence: 2 }
    ]);
  });
});

describe("Codex desktop input experiment", () => {
  let directory = "";
  let endpoint: CodexIpcEndpoint;
  let router: FakeCodexDesktopRouter;
  let owner: FakeCodexDesktopOwner | undefined;
  let client: CodexDesktopInputClient | undefined;

  beforeEach(async () => {
    directory = await mkdtemp(join(tmpdir(), "tokn-codex-ipc-test-"));
    if (process.platform === "win32") {
      endpoint = {
        transport: "windows_pipe",
        pipe_name: String.raw`\\.\pipe\tokn-codex-ipc-test-${process.pid}-${crypto.randomUUID()}`
      };
    } else {
      await mkdir(join(directory, "ipc"), { mode: 0o700 });
      endpoint = {
        transport: "unix_socket",
        path: join(directory, "ipc", "ipc.sock")
      };
    }
    router = new FakeCodexDesktopRouter(endpoint);
    await router.start();
  });

  afterEach(async () => {
    client?.close();
    owner?.close();
    await router.stop();
    await rm(directory, { recursive: true, force: true });
  });

  test("routes a start turn to the owner of the rollout conversation", async () => {
    owner = await FakeCodexDesktopOwner.connect({
      endpoint,
      conversation_id: "thread-lab-1",
      start_turn: () => ({ turn: { id: "turn-lab-1", status: "inProgress" } })
    });
    client = await CodexDesktopInputClient.connect({
      endpoint,
      timeout_ms: 5_000
    });

    const admission = await client.startTurn("thread-lab-1", "hello from Terminal Pet");

    expect(admission.conversation_id).toBe("thread-lab-1");
    expect(admission.result).toEqual({
      result: { turn: { id: "turn-lab-1", status: "inProgress" } }
    });
    expect(owner.last_start_turn?.params).toMatchObject({
      conversationId: "thread-lab-1",
      turnStartParams: {
        input: [{ type: "text", text: "hello from Terminal Pet" }]
      }
    });
  });

  test("forwards model and reasoning effort overrides", async () => {
    owner = await FakeCodexDesktopOwner.connect({
      endpoint,
      conversation_id: "thread-lab-settings",
      start_turn: () => ({ turn: { id: "turn-lab-settings", status: "inProgress" } })
    });
    client = await CodexDesktopInputClient.connect({
      endpoint,
      timeout_ms: 5_000
    });

    await client.startTurn("thread-lab-settings", "use luna", {
      model: "gpt-5.6-luna",
      effort: "low"
    });

    expect(owner.last_thread_settings?.params).toEqual({
      conversationId: "thread-lab-settings",
      threadSettings: {
        model: "gpt-5.6-luna",
        effort: "low"
      }
    });
    expect(owner.last_start_turn?.params.turnStartParams).toEqual({
      input: [{ type: "text", text: "use luna" }],
      clientUserMessageId: expect.any(String),
      additionalContext: null
    });
  });

  test.skipIf(process.platform === "win32")(
    "fails cleanly when no connected window owns the conversation",
    async () => {
    client = await CodexDesktopInputClient.connect({
      endpoint,
      timeout_ms: 5_000
    });

    await expect(client.startTurn("thread-lab-2", "hello")).rejects.toThrow(
      "no-client-found"
    );
    }
  );

  test.skipIf(process.platform === "win32")(
    "returns an owning window's routed error",
    async () => {
    owner = await FakeCodexDesktopOwner.connect({
      endpoint,
      conversation_id: "thread-lab-error",
      start_turn: () => {
        throw new Error("owner rejected test input");
      }
    });
    client = await CodexDesktopInputClient.connect({
      endpoint,
      timeout_ms: 5_000
    });

    await expect(client.startTurn("thread-lab-error", "hello")).rejects.toThrow(
      "owner rejected test input"
    );
    }
  );

  test("requires an explicit endpoint address", async () => {
    await expect(CodexDesktopInputClient.connect({
      endpoint: { transport: "unix_socket", path: "" }
    })).rejects.toThrow(
      "Unix socket path is required"
    );
  });
});

describe("Codex desktop IPC endpoint discovery", () => {
  test("uses CODEX_HOME for Unix desktop IPC", () => {
    expect(codexDesktopIpcEndpoint("/tmp/codex-lab", "darwin")).toEqual({
      transport: "unix_socket",
      path: "/tmp/codex-lab/ipc/ipc.sock"
    });
  });

  test("uses the Codex named pipe on Windows", () => {
    expect(codexDesktopIpcEndpoint("C:\\ignored", "win32")).toEqual({
      transport: "windows_pipe",
      pipe_name: CODEX_WINDOWS_PIPE_NAME
    });
  });

  test("rejects a Windows pipe outside the local named-pipe namespace", async () => {
    await expect(CodexDesktopInputClient.connect({
      endpoint: { transport: "windows_pipe", pipe_name: "codex-ipc" }
    })).rejects.toThrow("local named-pipe namespace");
  });
});
