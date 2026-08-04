import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { join } from "node:path";
import { tmpdir } from "node:os";

import { describe, expect, test } from "bun:test";

import {
  PiInputBroker,
  TerminalInputEditor,
  piInputAdmissionStatus,
  requestPiBridge,
} from "../src/input";
import {
  descriptorPathForSession,
  type PiInputBridgeDescriptor,
  type PiInputBridgeRequest,
} from "../../../extensions/pi/lib/input-protocol";
import { relayEvent } from "./fixtures";

const encoder = new TextEncoder();

describe("TerminalInputEditor", () => {
  test("collects text, edits it, and submits on Enter", () => {
    const editor = new TerminalInputEditor();

    expect(editor.begin()).toBe(true);
    expect(editor.feed(encoder.encode("hello"))).toEqual([
      { type: "changed" }
    ]);
    expect(editor.feed(Uint8Array.of(0x7f))).toEqual([
      { type: "changed" }
    ]);
    expect(editor.value).toBe("hell");
    expect(editor.feed(encoder.encode("o\r"))).toEqual([
      { type: "changed" },
      { type: "submitted", text: "hello" }
    ]);
    expect(editor.active).toBe(false);
  });

  test("cancels on Escape without submitting the draft", () => {
    const editor = new TerminalInputEditor();

    editor.begin();
    editor.feed(encoder.encode("draft"));
    expect(editor.feed(Uint8Array.of(0x1b))).toEqual([
      { type: "cancelled" }
    ]);
    expect(editor.value).toBe("");
    expect(editor.active).toBe(false);
  });

  test("keeps UTF-8 input intact across chunks", () => {
    const editor = new TerminalInputEditor();
    editor.begin();

    const bytes = encoder.encode("你好");
    expect(editor.feed(bytes.slice(0, 2))).toEqual([]);
    expect(editor.feed(bytes.slice(2))).toEqual([
      { type: "changed" }
    ]);
    expect(editor.value).toBe("你好");
  });
});

describe("PiInputBroker", () => {
  test("routes an observed Pi session through its live bridge", async () => {
    const directory = mkdtempSync(join(tmpdir(), "terminal-pet-input-"));
    try {
      const path = join(directory, "session.jsonl");
      writeFileSync(path, "{}\n");
      const descriptor = bridgeDescriptor(path);
      const descriptorPaths: string[] = [];
      const requests: PiInputBridgeRequest[] = [];
      const broker = new PiInputBroker({
        read_descriptor: async (descriptorPath) => {
          descriptorPaths.push(descriptorPath);
          return descriptor;
        },
        request: async (socketPath, request) => {
          expect(socketPath).toBe(descriptor.socket_path);
          requests.push(request);
          return {
            protocol: 1,
            type: "admitted",
            request_id: "request-1",
            session_id: "session-1",
            instance_id: "instance-1",
            disposition: "started"
          };
        },
        request_id: () => "request-1"
      });
      const event = relayEvent({ type: "message", role: "assistant" }, "pi.session-1");
      event.path = path;
      broker.observe(event);

      const admission = await broker.submit(event.topic, "  continue the task  ");

      expect(descriptorPaths).toEqual([descriptorPathForSession(path)]);
      expect(requests).toEqual([{
        protocol: 1,
        type: "submit",
        request_id: "request-1",
        token: "test-token",
        session_id: "session-1",
        session_file: path,
        instance_id: "instance-1",
        delivery: "auto",
        content: [{ type: "text", text: "continue the task" }]
      }]);
      expect(admission.disposition).toBe("started");
      expect(piInputAdmissionStatus(admission)).toBe("Pi accepted input");
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  test("rejects non-Pi sessions", async () => {
    const broker = new PiInputBroker();
    const event = relayEvent({ type: "message", role: "assistant" });
    broker.observe(event);

    await expect(broker.submit(event.topic, "continue")).rejects.toThrow(
      "observed Pi session"
    );
  });

  test("reports a missing live bridge without spawning Pi", async () => {
    const directory = mkdtempSync(join(tmpdir(), "terminal-pet-input-"));
    try {
      const path = join(directory, "session.jsonl");
      writeFileSync(path, "{}\n");
      const broker = new PiInputBroker({
        read_descriptor: async () => {
          throw new Error("descriptor not found");
        }
      });
      const event = relayEvent({ type: "message", role: "assistant" }, "pi.session-1");
      event.path = path;
      broker.observe(event);

      await expect(broker.submit(event.topic, "continue")).rejects.toThrow(
        "load the extension in the active Pi process"
      );
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  test("rejects descriptors for another process session", async () => {
    const directory = mkdtempSync(join(tmpdir(), "terminal-pet-input-"));
    try {
      const path = join(directory, "session.jsonl");
      writeFileSync(path, "{}\n");
      const descriptor = bridgeDescriptor(path);
      descriptor.session_id = "another-session";
      const broker = new PiInputBroker({
        read_descriptor: async () => descriptor
      });
      const event = relayEvent({ type: "message", role: "assistant" }, "pi.session-1");
      event.path = path;
      broker.observe(event);

      await expect(broker.submit(event.topic, "continue")).rejects.toThrow(
        "does not match the observed session"
      );
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  test("allows only one request per session at a time", async () => {
    const directory = mkdtempSync(join(tmpdir(), "terminal-pet-input-"));
    try {
      const path = join(directory, "session.jsonl");
      writeFileSync(path, "{}\n");
      const descriptor = bridgeDescriptor(path);
      let release!: (value: unknown) => void;
      const running = new Promise<unknown>((resolve) => {
        release = resolve;
      });
      const broker = new PiInputBroker({
        read_descriptor: async () => descriptor,
        request: async () => running,
        request_id: () => "request-in-flight"
      });
      const event = relayEvent({ type: "message", role: "assistant" }, "pi.session-1");
      event.path = path;
      broker.observe(event);

      const first = broker.submit(event.topic, "first");
      await expect(broker.submit(event.topic, "second")).rejects.toThrow(
        "input in flight"
      );
      release({
        protocol: 1,
        type: "admitted",
        request_id: "request-in-flight",
        session_id: "session-1",
        instance_id: "instance-1",
        disposition: "queued_follow_up"
      });
      await first;
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});

describe("requestPiBridge", () => {
  test("exchanges one JSONL request over a Unix socket", async () => {
    const directory = mkdtempSync(join(tmpdir(), "terminal-pet-input-socket-"));
    const socketPath = join(directory, "bridge.sock");
    const server = createServer((socket) => {
      socket.setEncoding("utf8");
      socket.once("data", (chunk: string) => {
        const request = JSON.parse(chunk.trim()) as PiInputBridgeRequest;
        socket.end(`${JSON.stringify({
          protocol: 1,
          type: "admitted",
          request_id: request.request_id,
          session_id: request.session_id,
          instance_id: request.instance_id,
          disposition: "started"
        })}\n`);
      });
    });
    try {
      await new Promise<void>((resolve, reject) => {
        server.once("error", reject);
        server.listen(socketPath, resolve);
      });
      const response = await requestPiBridge(socketPath, {
        protocol: 1,
        type: "submit",
        request_id: "socket-request",
        token: "test-token",
        session_id: "session-1",
        session_file: "/sessions/session.jsonl",
        instance_id: "instance-1",
        delivery: "auto",
        content: [{ type: "text", text: "continue" }]
      });

      expect(response).toMatchObject({
        type: "admitted",
        request_id: "socket-request",
        disposition: "started"
      });
    } finally {
      await new Promise<void>((resolve) => server.close(() => resolve()));
      rmSync(directory, { recursive: true, force: true });
    }
  });
});

function bridgeDescriptor(sessionFile: string): PiInputBridgeDescriptor {
  return {
    protocol: 1,
    provider: "pi",
    transport: "unix",
    session_id: "session-1",
    session_file: sessionFile,
    instance_id: "instance-1",
    socket_path: "/tmp/pi-input-test.sock",
    pid: 1234,
    token: "test-token"
  };
}
