import { mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

import { describe, expect, test } from "bun:test";

import {
  PiInputBroker,
  TerminalInputEditor,
  piCommand,
  type PiSessionInputRequest
} from "../src/input";
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
  test("keeps the prompt on stdin instead of the process argv", () => {
    expect(piCommand("/tmp/pi-session.jsonl")).toEqual([
      "pi",
      "--mode",
      "json",
      "--session",
      "/tmp/pi-session.jsonl",
      "--print"
    ]);
  });

  test("routes an observed Pi session to the direct Pi runner", async () => {
    const directory = mkdtempSync(join(tmpdir(), "terminal-pet-input-"));
    try {
      const path = join(directory, "session.jsonl");
      writeFileSync(path, "{}\n");
      const requests: PiSessionInputRequest[] = [];
      const broker = new PiInputBroker({
        run: async (request) => {
          requests.push(request);
        }
      });
      const event = relayEvent({ type: "message", role: "assistant" }, "pi.session-1");
      event.path = path;
      event.session.cwd = directory;
      broker.observe(event);

      await broker.submit(event.topic, "  continue the task  ");

      expect(requests).toEqual([{
        path: realpathSync(path),
        cwd: realpathSync(directory),
        prompt: "continue the task"
      }]);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  test("rejects non-Pi sessions", async () => {
    const broker = new PiInputBroker({ run: async () => {} });
    const event = relayEvent({ type: "message", role: "assistant" });
    broker.observe(event);

    await expect(broker.submit(event.topic, "continue")).rejects.toThrow(
      "observed Pi session"
    );
  });

  test("allows only one request per session at a time", async () => {
    const directory = mkdtempSync(join(tmpdir(), "terminal-pet-input-"));
    try {
      const path = join(directory, "session.jsonl");
      writeFileSync(path, "{}\n");
      let release!: () => void;
      const running = new Promise<void>((resolve) => {
        release = resolve;
      });
      const broker = new PiInputBroker({
        run: async () => running
      });
      const event = relayEvent({ type: "message", role: "assistant" }, "pi.session-1");
      event.path = path;
      broker.observe(event);

      const first = broker.submit(event.topic, "first");
      await expect(broker.submit(event.topic, "second")).rejects.toThrow(
        "input in flight"
      );
      release();
      await first;
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});
