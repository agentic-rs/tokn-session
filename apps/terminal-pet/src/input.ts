import { realpathSync, statSync } from "node:fs";
import { isAbsolute, resolve } from "node:path";

import type { RelayEvent } from "./protocol";

const MAX_INPUT_LENGTH = 16 * 1024;

export type TerminalInputEvent =
  | { type: "submitted"; text: string }
  | { type: "cancelled" }
  | { type: "changed" };

export class TerminalInputEditor {
  #active = false;
  #decoder = new TextDecoder();
  #value = "";

  get active(): boolean {
    return this.#active;
  }

  get value(): string {
    return this.#value;
  }

  begin(): boolean {
    if (this.#active) {
      return false;
    }
    this.#active = true;
    this.#value = "";
    this.#decoder = new TextDecoder();
    return true;
  }

  feed(chunk: Uint8Array): TerminalInputEvent[] {
    if (!this.#active) {
      return [];
    }

    const events: TerminalInputEvent[] = [];
    let textBytes: number[] = [];
    const flushText = (): void => {
      if (textBytes.length === 0) {
        return;
      }
      const text = this.#decoder.decode(Uint8Array.from(textBytes), {
        stream: true
      });
      textBytes = [];
      if (text.length === 0 || this.#value.length >= MAX_INPUT_LENGTH) {
        return;
      }
      const remaining = MAX_INPUT_LENGTH - this.#value.length;
      let addition = "";
      for (const character of text) {
        if (addition.length + character.length > remaining) {
          break;
        }
        addition += character;
      }
      if (addition.length === 0) {
        return;
      }
      this.#value += addition;
      events.push({ type: "changed" });
    };

    for (const byte of chunk) {
      if (byte === 0x0a || byte === 0x0d) {
        flushText();
        const text = this.#value;
        this.#reset();
        events.push({ type: "submitted", text });
        break;
      }
      if (byte === 0x1b || byte === 0x03) {
        flushText();
        this.#reset();
        events.push({ type: "cancelled" });
        break;
      }
      if (byte === 0x08 || byte === 0x7f) {
        flushText();
        const characters = Array.from(this.#value);
        characters.pop();
        this.#value = characters.join("");
        events.push({ type: "changed" });
        continue;
      }
      if (byte >= 0x20) {
        textBytes.push(byte);
      }
    }
    if (this.#active) {
      flushText();
    }
    return events;
  }

  cancel(): void {
    if (this.#active) {
      this.#reset();
    }
  }

  #reset(): void {
    this.#active = false;
    this.#value = "";
    this.#decoder.decode();
    this.#decoder = new TextDecoder();
  }
}

export interface PiSessionTarget {
  provider: string;
  path: string;
  cwd?: string | null;
}

export interface PiSessionInputRequest {
  path: string;
  cwd?: string;
  prompt: string;
}

export type PiSessionRunner = (
  request: PiSessionInputRequest
) => Promise<void>;

export interface PiInputBrokerOptions {
  run?: PiSessionRunner;
}

export function piCommand(path: string): string[] {
  return [
    "pi",
    "--mode",
    "json",
    "--session",
    path,
    "--print"
  ];
}

export class PiInputBroker {
  readonly #run: PiSessionRunner;
  readonly #targets = new Map<string, PiSessionTarget>();
  readonly #inFlight = new Map<string, Promise<void>>();

  constructor(options: PiInputBrokerOptions = {}) {
    this.#run = options.run ?? runPiSession;
  }

  observe(event: RelayEvent): void {
    const separator = event.topic.indexOf(".");
    const provider = event.session.provider
      ?? (separator > 0 ? event.topic.slice(0, separator) : undefined);
    if (provider?.toLowerCase() !== "pi" || !event.path) {
      return;
    }
    const previous = this.#targets.get(event.topic);
    this.#targets.set(event.topic, {
      provider: "pi",
      path: event.path,
      cwd: event.session.cwd ?? previous?.cwd
    });
  }

  async submit(topic: string, prompt: string): Promise<void> {
    const target = this.#targets.get(topic);
    if (!target || target.provider !== "pi") {
      throw new Error("input is only available for an observed Pi session");
    }

    const normalizedPrompt = normalizePrompt(prompt);
    const path = existingSessionPath(target.path);
    if (this.#inFlight.has(path)) {
      throw new Error("that Pi session already has input in flight");
    }

    const run = Promise.resolve().then(() => this.#run({
      path,
      cwd: existingDirectory(target.cwd),
      prompt: normalizedPrompt
    }));
    this.#inFlight.set(path, run);
    try {
      await run;
    } finally {
      if (this.#inFlight.get(path) === run) {
        this.#inFlight.delete(path);
      }
    }
  }
}

function normalizePrompt(prompt: string): string {
  const normalized = prompt.trim();
  if (normalized.length === 0) {
    throw new Error("message cannot be empty");
  }
  if (normalized.length > MAX_INPUT_LENGTH) {
    throw new Error("message is too long");
  }
  if (/[\u0000-\u001f\u007f]/u.test(normalized)) {
    throw new Error("message contains unsupported control characters");
  }
  return normalized;
}

function existingSessionPath(value: string): string {
  if (!isAbsolute(value)) {
    throw new Error("the observed Pi session path must be absolute");
  }
  const candidate = resolve(value);
  try {
    if (!statSync(candidate).isFile()) {
      throw new Error("not a regular file");
    }
    return realpathSync(candidate);
  } catch {
    throw new Error("the observed Pi session file is unavailable");
  }
}

function existingDirectory(value: string | null | undefined): string | undefined {
  if (!value || !isAbsolute(value)) {
    return undefined;
  }
  const candidate = resolve(value);
  try {
    return statSync(candidate).isDirectory() ? realpathSync(candidate) : undefined;
  } catch {
    return undefined;
  }
}

async function runPiSession(request: PiSessionInputRequest): Promise<void> {
  const child = Bun.spawn({
    cmd: piCommand(request.path),
    ...(request.cwd ? { cwd: request.cwd } : {}),
    stdin: "pipe",
    stdout: "ignore",
    stderr: "pipe"
  });
  const stderr = new Response(child.stderr).arrayBuffer();
  try {
    await child.stdin.write(request.prompt);
    await child.stdin.end();
  } catch (error) {
    child.kill("SIGTERM");
    await child.exited;
    throw error;
  }

  const [exitCode] = await Promise.all([child.exited, stderr]);
  if (exitCode !== 0) {
    throw new Error(`pi exited with status ${exitCode}`);
  }
}
