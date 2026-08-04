import { randomUUID } from "node:crypto";
import { statSync } from "node:fs";
import { lstat, readFile } from "node:fs/promises";
import { createConnection } from "node:net";
import { isAbsolute, resolve } from "node:path";

import {
  descriptorPathForSession,
  PI_INPUT_PROTOCOL_VERSION,
  PI_INPUT_PROVIDER,
  type PiInputBridgeDescriptor,
  type PiInputBridgeRequest,
  type PiInputBridgeResponse,
  type PiInputDelivery,
} from "../../../extensions/pi/lib/input-protocol";
import type { RelayEvent } from "./protocol";

const MAX_INPUT_LENGTH = 16 * 1024;
const MAX_RESPONSE_BYTES = 32 * 1024;
const BRIDGE_TIMEOUT_MS = 5_000;

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
  session_id: string;
}

export type PiInputAdmission = Extract<
  PiInputBridgeResponse,
  { type: "admitted" }
>;

export function piInputAdmissionStatus(admission: PiInputAdmission): string {
  switch (admission.disposition) {
    case "started":
      return "Pi accepted input";
    case "queued_follow_up":
      return "Pi queued follow-up input";
    case "queued_steer":
      return "Pi queued steering input";
  }
}

export type PiBridgeDescriptorReader = (path: string) => Promise<unknown>;
export type PiBridgeRequestSender = (
  socketPath: string,
  request: PiInputBridgeRequest
) => Promise<unknown>;

export interface PiInputBrokerOptions {
  read_descriptor?: PiBridgeDescriptorReader;
  request?: PiBridgeRequestSender;
  request_id?: () => string;
}

export class PiInputBroker {
  readonly #readDescriptor: PiBridgeDescriptorReader;
  readonly #request: PiBridgeRequestSender;
  readonly #requestId: () => string;
  readonly #targets = new Map<string, PiSessionTarget>();
  readonly #inFlight = new Map<string, Promise<PiInputAdmission>>();

  constructor(options: PiInputBrokerOptions = {}) {
    this.#readDescriptor = options.read_descriptor ?? readDescriptor;
    this.#request = options.request ?? requestPiBridge;
    this.#requestId = options.request_id ?? randomUUID;
  }

  observe(event: RelayEvent): void {
    const separator = event.topic.indexOf(".");
    const provider = event.session.provider
      ?? (separator > 0 ? event.topic.slice(0, separator) : undefined);
    if (provider?.toLowerCase() !== "pi" || !event.path) {
      return;
    }
    this.#targets.set(event.topic, {
      provider: "pi",
      path: event.path,
      session_id: event.session.session_id
    });
  }

  async submit(
    topic: string,
    prompt: string,
    delivery: PiInputDelivery = "auto"
  ): Promise<PiInputAdmission> {
    const target = this.#targets.get(topic);
    if (!target || target.provider !== "pi") {
      throw new Error("input is only available for an observed Pi session");
    }

    const normalizedPrompt = normalizePrompt(prompt);
    const sessionPath = existingSessionPath(target.path);
    if (this.#inFlight.has(sessionPath)) {
      throw new Error("that Pi session already has input in flight");
    }

    const run = this.#submitToBridge(
      target,
      sessionPath,
      normalizedPrompt,
      delivery
    );
    this.#inFlight.set(sessionPath, run);
    try {
      return await run;
    } finally {
      if (this.#inFlight.get(sessionPath) === run) {
        this.#inFlight.delete(sessionPath);
      }
    }
  }

  async #submitToBridge(
    target: PiSessionTarget,
    sessionPath: string,
    prompt: string,
    delivery: PiInputDelivery
  ): Promise<PiInputAdmission> {
    const descriptorPath = descriptorPathForSession(sessionPath);
    let descriptorValue: unknown;
    try {
      descriptorValue = await this.#readDescriptor(descriptorPath);
    } catch (error) {
      throw bridgeUnavailable(error);
    }
    const descriptor = parseDescriptor(descriptorValue, target, sessionPath);
    const requestId = this.#requestId();
    const request: PiInputBridgeRequest = {
      protocol: PI_INPUT_PROTOCOL_VERSION,
      type: "submit",
      request_id: requestId,
      token: descriptor.token,
      session_id: descriptor.session_id,
      session_file: descriptor.session_file,
      instance_id: descriptor.instance_id,
      delivery,
      content: [{ type: "text", text: prompt }]
    };

    let responseValue: unknown;
    try {
      responseValue = await this.#request(descriptor.socket_path, request);
    } catch (error) {
      throw bridgeUnavailable(error);
    }
    return parseAdmission(responseValue, descriptor, requestId);
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
    return candidate;
  } catch {
    throw new Error("the observed Pi session file is unavailable");
  }
}

async function readDescriptor(path: string): Promise<unknown> {
  const info = await lstat(path);
  if (!info.isFile()) {
    throw new Error("descriptor is not a regular file");
  }
  if (info.size > MAX_RESPONSE_BYTES) {
    throw new Error("descriptor is too large");
  }
  if ((info.mode & 0o077) !== 0) {
    throw new Error("descriptor permissions are not private");
  }
  const currentUid = process.getuid?.();
  if (currentUid !== undefined && info.uid !== currentUid) {
    throw new Error("descriptor is owned by another user");
  }
  return JSON.parse(await readFile(path, "utf8")) as unknown;
}

function parseDescriptor(
  value: unknown,
  target: PiSessionTarget,
  sessionPath: string
): PiInputBridgeDescriptor {
  const descriptor = asRecord(value);
  if (
    !descriptor
    || descriptor.protocol !== PI_INPUT_PROTOCOL_VERSION
    || descriptor.provider !== PI_INPUT_PROVIDER
    || descriptor.transport !== "unix"
    || descriptor.session_id !== target.session_id
    || descriptor.session_file !== sessionPath
    || !isNonEmptyString(descriptor.instance_id)
    || !isNonEmptyString(descriptor.socket_path)
    || !isAbsolute(descriptor.socket_path)
    || typeof descriptor.pid !== "number"
    || !Number.isInteger(descriptor.pid)
    || descriptor.pid <= 0
    || !isNonEmptyString(descriptor.token)
  ) {
    throw new Error("Pi input bridge descriptor does not match the observed session");
  }
  return {
    protocol: PI_INPUT_PROTOCOL_VERSION,
    provider: PI_INPUT_PROVIDER,
    transport: "unix",
    session_id: descriptor.session_id,
    session_file: descriptor.session_file,
    instance_id: descriptor.instance_id,
    socket_path: descriptor.socket_path,
    pid: descriptor.pid,
    token: descriptor.token
  };
}

function parseAdmission(
  value: unknown,
  descriptor: PiInputBridgeDescriptor,
  requestId: string
): PiInputAdmission {
  const response = asRecord(value);
  if (!response || response.protocol !== PI_INPUT_PROTOCOL_VERSION) {
    throw new Error("Pi input bridge returned an invalid response");
  }
  if (response.type === "error") {
    if (
      response.request_id !== undefined
      && response.request_id !== requestId
    ) {
      throw new Error("Pi input bridge returned a mismatched error response");
    }
    const message = isNonEmptyString(response.message)
      ? response.message
      : "input was rejected";
    throw new Error(`Pi input bridge rejected the message: ${message}`);
  }
  if (
    response.type !== "admitted"
    || response.request_id !== requestId
    || response.session_id !== descriptor.session_id
    || response.instance_id !== descriptor.instance_id
    || !isDisposition(response.disposition)
  ) {
    throw new Error("Pi input bridge returned a mismatched admission");
  }
  return {
    protocol: PI_INPUT_PROTOCOL_VERSION,
    type: "admitted",
    request_id: response.request_id,
    session_id: response.session_id,
    instance_id: response.instance_id,
    disposition: response.disposition
  };
}

export function requestPiBridge(
  socketPath: string,
  request: PiInputBridgeRequest
): Promise<unknown> {
  return new Promise((resolveRequest, rejectRequest) => {
    const socket = createConnection(socketPath);
    let settled = false;
    let buffer = "";

    const finish = (error?: unknown, value?: unknown): void => {
      if (settled) {
        return;
      }
      settled = true;
      socket.removeAllListeners();
      socket.destroy();
      if (error !== undefined) {
        rejectRequest(error);
      } else {
        resolveRequest(value);
      }
    };

    socket.setEncoding("utf8");
    socket.setTimeout(BRIDGE_TIMEOUT_MS, () => {
      finish(new Error("Pi input bridge timed out"));
    });
    socket.once("connect", () => {
      socket.write(`${JSON.stringify(request)}\n`);
    });
    socket.on("data", (chunk: string) => {
      buffer += chunk;
      if (Buffer.byteLength(buffer, "utf8") > MAX_RESPONSE_BYTES) {
        finish(new Error("Pi input bridge response is too large"));
        return;
      }
      const newline = buffer.indexOf("\n");
      if (newline < 0) {
        return;
      }
      try {
        finish(undefined, JSON.parse(buffer.slice(0, newline)) as unknown);
      } catch {
        finish(new Error("Pi input bridge returned invalid JSON"));
      }
    });
    socket.once("end", () => {
      finish(new Error("Pi input bridge closed without a response"));
    });
    socket.once("error", (error) => finish(error));
  });
}

function bridgeUnavailable(error: unknown): Error {
  const detail = error instanceof Error ? error.message : String(error);
  return new Error(
    `Pi input bridge is unavailable for this session; load the extension in the active Pi process (${detail})`
  );
}

function isDisposition(value: unknown): value is PiInputAdmission["disposition"] {
  return value === "started"
    || value === "queued_follow_up"
    || value === "queued_steer";
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}
