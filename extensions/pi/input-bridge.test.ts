import { createConnection } from "node:net";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";

import {
  PiInputBridge,
  type PiInputBridgeDescriptor,
  type PiInputBridgeResponse,
  type PiExtensionContext
} from "./input-bridge";

interface TestHarness {
  context: PiExtensionContext;
  descriptor: PiInputBridgeDescriptor;
  bridge: PiInputBridge;
  getIdle(): boolean;
  setIdle(value: boolean): void;
  messages: string[];
}

async function makeHarness(root: string): Promise<TestHarness> {
  const sessionFile = join(root, "session.jsonl");
  const descriptorPath = join(root, "descriptor.json");
  const socketPath = join(root, "bridge.sock");
  await writeFile(sessionFile, "{\"type\":\"session\",\"id\":\"session-1\"}\n");

  let idle = true;
  const messages: string[] = [];
  const context: PiExtensionContext = {
    mode: "tui",
    isIdle: () => idle,
    sessionManager: {
      getSessionFile: () => sessionFile,
      getSessionId: () => "session-1"
    },
    ui: {
      notify: () => {},
      setStatus: () => {}
    }
  };
  const bridge = new PiInputBridge({
    api: {
      sendUserMessage(message) {
        messages.push(message);
        idle = false;
      }
    },
    context,
    descriptor_path: descriptorPath,
    socket_path: socketPath,
    token: "test-token",
    pid: 1234
  });
  const descriptor = await bridge.start();
  return {
    context,
    descriptor,
    bridge,
    getIdle: () => idle,
    setIdle: (value) => {
      idle = value;
    },
    messages
  };
}

function request(socketPath: string, value: unknown): Promise<PiInputBridgeResponse> {
  return new Promise((resolveRequest, rejectRequest) => {
    const socket = createConnection(socketPath);
    let output = "";
    socket.setEncoding("utf8");
    socket.once("connect", () => {
      socket.write(`${JSON.stringify(value)}\n`);
    });
    socket.on("data", (chunk: string) => {
      output += chunk;
    });
    socket.once("end", () => {
      try {
        resolveRequest(JSON.parse(output) as PiInputBridgeResponse);
      } catch (error) {
        rejectRequest(error);
      }
    });
    socket.once("error", rejectRequest);
  });
}

describe("Pi input bridge", () => {
  test("publishes a session descriptor and routes prompts to Pi", async () => {
    const root = await mkdtemp(join(tmpdir(), "tokn-pi-input-test-"));
    const harness = await makeHarness(root);
    try {
      const descriptor = JSON.parse(await readFile(join(root, "descriptor.json"), "utf8")) as PiInputBridgeDescriptor;
      expect(descriptor).toEqual(harness.descriptor);
      expect(descriptor).toMatchObject({
        protocol: 1,
        transport: "unix",
        session_id: "session-1",
        pid: 1234,
        token: "test-token"
      });

      await expect(
        request(harness.descriptor.socket_path, {
          protocol: 1,
          type: "status",
          request_id: "status-1",
          token: "test-token",
          session_id: "session-1",
          session_file: harness.descriptor.session_file
        })
      ).resolves.toMatchObject({
        protocol: 1,
        type: "ready",
        request_id: "status-1",
        state: "idle"
      });

      await expect(
        request(harness.descriptor.socket_path, {
          protocol: 1,
          type: "prompt",
          request_id: "prompt-1",
          token: "test-token",
          session_id: "session-1",
          session_file: harness.descriptor.session_file,
          message: "  Continue the task  "
        })
      ).resolves.toMatchObject({
        protocol: 1,
        type: "accepted",
        request_id: "prompt-1",
        session_id: "session-1"
      });
      expect(harness.messages).toEqual(["Continue the task"]);
      expect(harness.getIdle()).toBe(false);

      await expect(
        request(harness.descriptor.socket_path, {
          protocol: 1,
          type: "prompt",
          token: "test-token",
          session_id: "session-1",
          session_file: harness.descriptor.session_file,
          message: "One more"
        })
      ).resolves.toMatchObject({
        protocol: 1,
        type: "error",
        code: "busy"
      });
    } finally {
      await harness.bridge.stop();
      try {
        await expect(readFile(join(root, "descriptor.json"))).rejects.toMatchObject({ code: "ENOENT" });
      } finally {
        await rm(root, { recursive: true, force: true });
      }
    }
  });

  test("rejects unauthorized and mismatched session requests", async () => {
    const root = await mkdtemp(join(tmpdir(), "tokn-pi-input-test-"));
    const harness = await makeHarness(root);
    try {
      await expect(
        request(harness.descriptor.socket_path, {
          protocol: 1,
          type: "status",
          token: "wrong-token",
          session_id: "session-1",
          session_file: harness.descriptor.session_file
        })
      ).resolves.toMatchObject({ type: "error", code: "unauthorized" });

      await expect(
        request(harness.descriptor.socket_path, {
          protocol: 1,
          type: "status",
          token: "test-token",
          session_id: "other-session",
          session_file: harness.descriptor.session_file
        })
      ).resolves.toMatchObject({ type: "error", code: "session_mismatch" });
    } finally {
      await harness.bridge.stop();
      await rm(root, { recursive: true, force: true });
    }
  });
});
