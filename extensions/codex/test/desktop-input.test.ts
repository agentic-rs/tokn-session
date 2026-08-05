import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { CodexDesktopInputClient } from "../lib/desktop-input-client";
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
  let socketPath = "";
  let router: FakeCodexDesktopRouter;
  let owner: FakeCodexDesktopOwner | undefined;
  let client: CodexDesktopInputClient | undefined;

  beforeEach(async () => {
    directory = await mkdtemp(join(tmpdir(), "tokn-codex-ipc-test-"));
    await mkdir(join(directory, "ipc"), { mode: 0o700 });
    socketPath = join(directory, "ipc", "ipc.sock");
    router = new FakeCodexDesktopRouter(socketPath);
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
      socket_path: socketPath,
      conversation_id: "thread-lab-1",
      start_turn: () => ({ turn: { id: "turn-lab-1", status: "inProgress" } })
    });
    client = await CodexDesktopInputClient.connect({
      socket_path: socketPath,
      timeout_ms: 1_000
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

  test("fails cleanly when no connected window owns the conversation", async () => {
    owner = await FakeCodexDesktopOwner.connect({
      socket_path: socketPath,
      conversation_id: "another-thread"
    });
    client = await CodexDesktopInputClient.connect({
      socket_path: socketPath,
      timeout_ms: 1_000
    });

    await expect(client.startTurn("thread-lab-2", "hello")).rejects.toThrow(
      "no-client-found"
    );
  });

  test("requires an explicit socket path", async () => {
    await expect(CodexDesktopInputClient.connect({ socket_path: "" })).rejects.toThrow(
      "explicit Codex desktop IPC socket path"
    );
  });
});
