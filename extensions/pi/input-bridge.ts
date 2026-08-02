import { createHash, randomBytes } from "node:crypto";
import {
  chmod,
  lstat,
  mkdir,
  readFile,
  rename,
  unlink,
  writeFile
} from "node:fs/promises";
import { createServer, createConnection, type Server, type Socket } from "node:net";
import { dirname, join, resolve } from "node:path";
import { tmpdir } from "node:os";

const PROTOCOL_VERSION = 1;
const DESCRIPTOR_SUFFIX = ".tokn-input.json";
const SOCKET_DIRECTORY = "tokn-pi-input";
const MAX_FRAME_BYTES = 32 * 1024;
const MAX_MESSAGE_LENGTH = 16 * 1024;
const SOCKET_REQUEST_TIMEOUT_MS = 5_000;

type SessionStartReason = "startup" | "reload" | "new" | "resume" | "fork";
type SessionShutdownReason = "quit" | "reload" | "new" | "resume" | "fork";

export interface PiSessionStartEvent {
  reason: SessionStartReason;
  previousSessionFile?: string;
}

export interface PiSessionShutdownEvent {
  reason: SessionShutdownReason;
  targetSessionFile?: string;
}

export interface PiSessionManager {
  getSessionFile(): string | undefined;
  getSessionId(): string;
}

export interface PiExtensionUi {
  notify(message: string, type?: "info" | "warning" | "error"): void;
  setStatus(key: string, text: string | undefined): void;
}

export interface PiExtensionContext {
  mode: string;
  isIdle(): boolean;
  sessionManager: PiSessionManager;
  ui: PiExtensionUi;
}

export interface PiExtensionApi {
  on(
    event: "session_start",
    handler: (event: PiSessionStartEvent, context: PiExtensionContext) => void | Promise<void>
  ): void;
  on(
    event: "session_shutdown",
    handler: (event: PiSessionShutdownEvent, context: PiExtensionContext) => void | Promise<void>
  ): void;
  sendUserMessage(
    content: string,
    options?: { deliverAs?: "steer" | "followUp" }
  ): void;
}

export interface PiInputBridgeDescriptor {
  protocol: typeof PROTOCOL_VERSION;
  transport: "unix";
  session_id: string;
  session_file: string;
  socket_path: string;
  pid: number;
  token: string;
}

export type PiInputBridgeRequest =
  | {
      protocol: typeof PROTOCOL_VERSION;
      type: "status";
      request_id?: string;
      token: string;
      session_id: string;
      session_file: string;
    }
  | {
      protocol: typeof PROTOCOL_VERSION;
      type: "prompt";
      request_id?: string;
      token: string;
      session_id: string;
      session_file: string;
      message: string;
    };

export type PiInputBridgeResponse =
  | {
      protocol: typeof PROTOCOL_VERSION;
      type: "ready";
      request_id?: string;
      session_id: string;
      session_file: string;
      state: "idle" | "busy";
    }
  | {
      protocol: typeof PROTOCOL_VERSION;
      type: "accepted";
      request_id?: string;
      session_id: string;
    }
  | {
      protocol: typeof PROTOCOL_VERSION;
      type: "error";
      request_id?: string;
      code:
        | "bridge_unavailable"
        | "busy"
        | "invalid_request"
        | "message_invalid"
        | "session_mismatch"
        | "unauthorized"
        | "unsupported";
      message: string;
    };

export interface PiInputBridgeOptions {
  api: Pick<PiExtensionApi, "sendUserMessage">;
  context: PiExtensionContext;
  descriptor_path?: string;
  socket_path?: string;
  token?: string;
  pid?: number;
}

export function descriptorPathForSession(sessionFile: string): string {
  return `${resolve(sessionFile)}${DESCRIPTOR_SUFFIX}`;
}

export function socketPathForSession(sessionFile: string): string {
  const digest = createHash("sha256").update(resolve(sessionFile)).digest("hex").slice(0, 24);
  return join(tmpdir(), SOCKET_DIRECTORY, `${digest}.sock`);
}

export class PiInputBridge {
  readonly #api: Pick<PiExtensionApi, "sendUserMessage">;
  readonly #context: PiExtensionContext;
  readonly #descriptorPathOverride: string | undefined;
  readonly #socketPathOverride: string | undefined;
  readonly #tokenOverride: string | undefined;
  readonly #pid: number;
  #server: Server | undefined;
  #descriptor: PiInputBridgeDescriptor | undefined;
  #descriptorPath: string | undefined;
  readonly #connections = new Set<Socket>();

  constructor(options: PiInputBridgeOptions) {
    this.#api = options.api;
    this.#context = options.context;
    this.#descriptorPathOverride = options.descriptor_path;
    this.#socketPathOverride = options.socket_path;
    this.#tokenOverride = options.token;
    this.#pid = options.pid ?? process.pid;
  }

  get descriptor(): PiInputBridgeDescriptor | undefined {
    return this.#descriptor;
  }

  async start(): Promise<PiInputBridgeDescriptor> {
    if (this.#descriptor) {
      return this.#descriptor;
    }
    if (process.platform === "win32") {
      throw new Error("the Pi input bridge currently requires Unix domain sockets");
    }
    if (this.#context.mode !== "tui") {
      throw new Error("the Pi input bridge is only available in interactive mode");
    }

    const sessionFile = this.#context.sessionManager.getSessionFile();
    if (!sessionFile) {
      throw new Error("the Pi input bridge requires a persisted session");
    }

    const sessionPath = resolve(sessionFile);
    const descriptorPath = this.#descriptorPathOverride ?? descriptorPathForSession(sessionPath);
    const socketPath = this.#socketPathOverride ?? socketPathForSession(sessionPath);
    const descriptor: PiInputBridgeDescriptor = {
      protocol: PROTOCOL_VERSION,
      transport: "unix",
      session_id: this.#context.sessionManager.getSessionId(),
      session_file: sessionPath,
      socket_path: socketPath,
      pid: this.#pid,
      token: this.#tokenOverride ?? randomBytes(32).toString("hex")
    };

    await prepareSocketPath(socketPath);
    await mkdir(dirname(descriptorPath), { recursive: true, mode: 0o700 });

    const server = createServer((socket) => {
      this.#connections.add(socket);
      socket.once("close", () => this.#connections.delete(socket));
      this.#handleSocket(socket);
    });

    try {
      await listen(server, socketPath);
      server.unref();
      await chmod(socketPath, 0o600);
      await writeDescriptor(descriptorPath, descriptor);
    } catch (error) {
      for (const socket of this.#connections) {
        socket.destroy();
      }
      await closeServer(server, socketPath);
      throw error;
    }

    this.#server = server;
    this.#descriptor = descriptor;
    this.#descriptorPath = descriptorPath;
    return descriptor;
  }

  async stop(): Promise<void> {
    const descriptor = this.#descriptor;
    const server = this.#server;
    const descriptorPath = this.#descriptorPath;
    this.#descriptor = undefined;
    this.#server = undefined;
    this.#descriptorPath = undefined;

    if (descriptor) {
      try {
        if (server) {
          for (const socket of this.#connections) {
            socket.destroy();
          }
          await closeServer(server, descriptor.socket_path);
        }
      } finally {
        await removeDescriptor(descriptorPath ?? descriptorPathForSession(descriptor.session_file), descriptor);
      }
    }
  }

  #handleSocket(socket: Socket): void {
    socket.setEncoding("utf8");
    socket.setTimeout(SOCKET_REQUEST_TIMEOUT_MS, () => socket.destroy());

    let buffer = "";
    const onData = (chunk: string): void => {
      buffer += chunk;
      if (Buffer.byteLength(buffer, "utf8") > MAX_FRAME_BYTES) {
        socket.off("data", onData);
        socket.end(encodeResponse({
          protocol: PROTOCOL_VERSION,
          type: "error",
          code: "invalid_request",
          message: "request frame is too large"
        }));
        return;
      }

      const newline = buffer.indexOf("\n");
      if (newline < 0) {
        return;
      }
      socket.off("data", onData);
      const line = buffer.slice(0, newline).replace(/\r$/u, "");
      this.#respond(socket, line);
    };
    socket.on("data", onData);
    socket.on("error", () => {
      socket.destroy();
    });
  }

  #respond(socket: Socket, line: string): void {
    const response = this.#handleRequest(line);
    socket.end(encodeResponse(response));
  }

  #handleRequest(line: string): PiInputBridgeResponse {
    let value: unknown;
    try {
      value = JSON.parse(line);
    } catch {
      return errorResponse("invalid_request", "request is not valid JSON");
    }

    if (!isRecord(value) || value.protocol !== PROTOCOL_VERSION || typeof value.type !== "string") {
      return errorResponse("invalid_request", "request has an unsupported protocol");
    }

    const requestId = asOptionalString(value.request_id);
    const token = asOptionalString(value.token);
    const sessionId = asOptionalString(value.session_id);
    const sessionFile = asOptionalString(value.session_file);
    const descriptor = this.#descriptor;
    if (!descriptor || !token || token !== descriptor.token) {
      return errorResponse("unauthorized", "bridge capability token is invalid", requestId);
    }
    if (!sessionId || !sessionFile || sessionId !== descriptor.session_id || resolve(sessionFile) !== descriptor.session_file) {
      return errorResponse("session_mismatch", "request does not target the active Pi session", requestId);
    }

    if (value.type === "status") {
      return {
        protocol: PROTOCOL_VERSION,
        type: "ready",
        ...(requestId ? { request_id: requestId } : {}),
        session_id: descriptor.session_id,
        session_file: descriptor.session_file,
        state: this.#context.isIdle() ? "idle" : "busy"
      };
    }

    if (value.type !== "prompt") {
      return errorResponse("invalid_request", "request type is unsupported", requestId);
    }

    const message = normalizeMessage(value.message);
    if (!message) {
      return errorResponse("message_invalid", "message must be non-empty and contain no control characters", requestId);
    }
    if (!this.#context.isIdle()) {
      return errorResponse("busy", "Pi is already processing a turn", requestId);
    }

    try {
      this.#api.sendUserMessage(message);
    } catch (error) {
      return errorResponse("bridge_unavailable", errorMessage(error), requestId);
    }

    return {
      protocol: PROTOCOL_VERSION,
      type: "accepted",
      ...(requestId ? { request_id: requestId } : {}),
      session_id: descriptor.session_id
    };
  }
}

export default function installPiInputBridge(pi: PiExtensionApi): void {
  let bridge: PiInputBridge | undefined;

  pi.on("session_start", async (_event, context) => {
    await bridge?.stop();
    bridge = undefined;
    if (context.mode !== "tui" || !context.sessionManager.getSessionFile()) {
      return;
    }

    try {
      bridge = new PiInputBridge({ api: pi, context });
      await bridge.start();
      context.ui.setStatus("tokn-input", "input bridge ready");
    } catch (error) {
      context.ui.setStatus("tokn-input", "input bridge unavailable");
      context.ui.notify(`Pi input bridge unavailable: ${errorMessage(error)}`, "warning");
    }
  });

  pi.on("session_shutdown", async (_event, context) => {
    await bridge?.stop();
    bridge = undefined;
    context.ui.setStatus("tokn-input", undefined);
  });
}

async function prepareSocketPath(socketPath: string): Promise<void> {
  await mkdir(dirname(socketPath), { recursive: true, mode: 0o700 });
  try {
    const info = await lstat(socketPath);
    if (!info.isSocket()) {
      throw new Error(`bridge socket path is occupied: ${socketPath}`);
    }
  } catch (error) {
    if (isNodeError(error, "ENOENT")) {
      return;
    }
    throw error;
  }

  const live = await probeSocket(socketPath);
  if (live) {
    throw new Error("another Pi input bridge is already active for this session");
  }
  await unlink(socketPath).catch((error: unknown) => {
    if (!isNodeError(error, "ENOENT")) {
      throw error;
    }
  });
}

function probeSocket(socketPath: string): Promise<boolean> {
  return new Promise((resolveProbe, rejectProbe) => {
    const socket = createConnection(socketPath);
    let settled = false;
    const finish = (result: boolean): void => {
      if (settled) {
        return;
      }
      settled = true;
      socket.removeAllListeners();
      socket.destroy();
      resolveProbe(result);
    };
    socket.once("connect", () => finish(true));
    socket.once("error", (error: unknown) => {
      if (isNodeError(error, "ENOENT") || isNodeError(error, "ECONNREFUSED")) {
        finish(false);
        return;
      }
      if (settled) {
        return;
      }
      settled = true;
      socket.removeAllListeners();
      socket.destroy();
      rejectProbe(error);
    });
  });
}

function listen(server: Server, socketPath: string): Promise<void> {
  return new Promise((resolveListen, rejectListen) => {
    const onError = (error: Error): void => {
      server.off("listening", onListening);
      rejectListen(error);
    };
    const onListening = (): void => {
      server.off("error", onError);
      resolveListen();
    };
    server.once("error", onError);
    server.once("listening", onListening);
    server.listen(socketPath);
  });
}

async function closeServer(server: Server, socketPath: string): Promise<void> {
  await new Promise<void>((resolveClose, rejectClose) => {
    server.close((error) => {
      if (error) {
        rejectClose(error);
        return;
      }
      resolveClose();
    });
  }).catch((error: unknown) => {
    if (!isNodeError(error, "ERR_SERVER_NOT_RUNNING")) {
      throw error;
    }
  });
  await unlink(socketPath).catch((error: unknown) => {
    if (!isNodeError(error, "ENOENT")) {
      throw error;
    }
  });
}

async function writeDescriptor(path: string, descriptor: PiInputBridgeDescriptor): Promise<void> {
  const temporaryPath = `${path}.${process.pid}.${randomBytes(4).toString("hex")}.tmp`;
  try {
    await writeFile(temporaryPath, `${JSON.stringify(descriptor)}\n`, { mode: 0o600 });
    await chmod(temporaryPath, 0o600);
    await rename(temporaryPath, path);
  } catch (error) {
    await unlink(temporaryPath).catch(() => {});
    throw error;
  }
}

async function removeDescriptor(path: string, descriptor: PiInputBridgeDescriptor): Promise<void> {
  try {
    const current = JSON.parse(await readFile(path, "utf8")) as Partial<PiInputBridgeDescriptor>;
    if (current.token !== descriptor.token || current.socket_path !== descriptor.socket_path) {
      return;
    }
  } catch (error) {
    if (isNodeError(error, "ENOENT")) {
      return;
    }
    return;
  }
  await unlink(path).catch((error: unknown) => {
    if (!isNodeError(error, "ENOENT")) {
      throw error;
    }
  });
}

function normalizeMessage(value: unknown): string | undefined {
  if (typeof value !== "string") {
    return undefined;
  }
  const message = value.trim();
  if (
    message.length === 0
    || message.length > MAX_MESSAGE_LENGTH
    || /[\u0000-\u001f\u007f]/u.test(message)
  ) {
    return undefined;
  }
  return message;
}

function errorResponse(
  code: Extract<PiInputBridgeResponse, { type: "error" }>["code"],
  message: string,
  requestId?: string
): PiInputBridgeResponse {
  return {
    protocol: PROTOCOL_VERSION,
    type: "error",
    ...(requestId ? { request_id: requestId } : {}),
    code,
    message
  };
}

function encodeResponse(response: PiInputBridgeResponse): string {
  return `${JSON.stringify(response)}\n`;
}

function asOptionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNodeError(value: unknown, code: string): value is NodeJS.ErrnoException {
  return typeof value === "object" && value !== null && "code" in value && value.code === code;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
