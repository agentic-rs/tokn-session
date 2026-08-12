import { randomUUID } from "node:crypto";
import { chmod, lstat, unlink } from "node:fs/promises";
import { createConnection, createServer, type Server, type Socket } from "node:net";

import {
  ipcEndpointAddress,
  type CodexIpcEndpoint,
} from "../lib/ipc-endpoint";

import {
  CODEX_DESKTOP_START_TURN_VERSION,
  encodeIpcFrame,
  IpcFrameDecoder,
  isRecord,
  type CodexDesktopClientDiscoveryRequest,
  type CodexDesktopClientDiscoveryResponse,
  type CodexDesktopStartTurnRequest,
  type CodexDesktopUpdateThreadSettingsRequest,
} from "../lib/ipc-protocol";

interface RouterPeer {
  socket: Socket;
  decoder: IpcFrameDecoder;
  client_id?: string;
}

interface DiscoveryGroup {
  source: RouterPeer;
  request: CodexDesktopStartTurnRequest | CodexDesktopUpdateThreadSettingsRequest;
  pending_ids: Set<string>;
  settled: boolean;
}

interface PendingRoute {
  source: RouterPeer;
  target: RouterPeer;
}

export class FakeCodexDesktopRouter {
  readonly #endpoint: CodexIpcEndpoint;
  readonly #address: string;
  readonly #peers = new Set<RouterPeer>();
  readonly #discoveries = new Map<string, DiscoveryGroup>();
  readonly #routes = new Map<string, PendingRoute>();
  #server: Server | undefined;

  constructor(endpoint: CodexIpcEndpoint) {
    this.#endpoint = endpoint;
    this.#address = ipcEndpointAddress(endpoint);
  }

  async start(): Promise<void> {
    if (this.#server) {
      return;
    }
    await prepareEndpoint(this.#endpoint);
    const server = createServer((socket) => this.#accept(socket));
    await listen(server, this.#address);
    await secureEndpoint(this.#endpoint);
    this.#server = server;
  }

  async stop(): Promise<void> {
    const server = this.#server;
    this.#server = undefined;
    for (const peer of this.#peers) {
      peer.socket.destroy();
    }
    this.#peers.clear();
    this.#discoveries.clear();
    this.#routes.clear();
    if (server) {
      await close(server);
    }
    await removeEndpoint(this.#endpoint);
  }

  #accept(socket: Socket): void {
    const peer: RouterPeer = { socket, decoder: new IpcFrameDecoder() };
    this.#peers.add(peer);
    socket.on("data", (chunk: Buffer) => {
      try {
        for (const message of peer.decoder.push(chunk)) {
          this.#handle(peer, message);
        }
      } catch {
        socket.destroy();
      }
    });
    socket.on("close", () => this.#peers.delete(peer));
    socket.on("error", () => socket.destroy());
  }

  #handle(peer: RouterPeer, message: unknown): void {
    if (!isRecord(message) || typeof message.type !== "string") {
      return;
    }
    if (message.type === "request") {
      if (message.method === "initialize") {
        this.#initialize(peer, message);
        return;
      }
      if (isRoutableThreadRequest(message)) {
        this.#discoverOwner(peer, message);
      }
      return;
    }
    if (message.type === "client-discovery-response") {
      this.#handleDiscovery(peer, message);
      return;
    }
    if (message.type === "response" && typeof message.requestId === "string") {
      const route = this.#routes.get(message.requestId);
      if (!route || route.target !== peer) {
        return;
      }
      this.#routes.delete(message.requestId);
      route.source.socket.write(encodeIpcFrame(message));
    }
  }

  #initialize(peer: RouterPeer, message: Record<string, unknown>): void {
    if (typeof message.requestId !== "string") {
      return;
    }
    peer.client_id ??= randomUUID();
    peer.socket.write(encodeIpcFrame({
      type: "response",
      requestId: message.requestId,
      resultType: "success",
      method: "initialize",
      handledByClientId: peer.client_id,
      result: { clientId: peer.client_id }
    }));
  }

  #discoverOwner(
    source: RouterPeer,
    request: CodexDesktopStartTurnRequest | CodexDesktopUpdateThreadSettingsRequest
  ): void {
    const candidates = [...this.#peers].filter((peer) => peer !== source && peer.client_id);
    if (candidates.length === 0) {
      source.socket.write(encodeIpcFrame(errorResponse(request.requestId, "no-client-found")));
      return;
    }
    const group: DiscoveryGroup = {
      source,
      request,
      pending_ids: new Set(),
      settled: false
    };
    for (const candidate of candidates) {
      const discoveryId = randomUUID();
      group.pending_ids.add(discoveryId);
      this.#discoveries.set(discoveryId, group);
      const discovery: CodexDesktopClientDiscoveryRequest = {
        type: "client-discovery-request",
        requestId: discoveryId,
        request
      };
      candidate.socket.write(encodeIpcFrame(discovery));
    }
  }

  #handleDiscovery(peer: RouterPeer, message: Record<string, unknown>): void {
    if (
      typeof message.requestId !== "string"
      || !isRecord(message.response)
      || typeof message.response.canHandle !== "boolean"
    ) {
      return;
    }
    const group = this.#discoveries.get(message.requestId);
    if (!group || group.settled) {
      return;
    }
    this.#discoveries.delete(message.requestId);
    group.pending_ids.delete(message.requestId);
    if (message.response.canHandle) {
      group.settled = true;
      for (const pendingId of group.pending_ids) {
        this.#discoveries.delete(pendingId);
      }
      this.#routes.set(group.request.requestId, { source: group.source, target: peer });
      peer.socket.write(encodeIpcFrame(group.request));
      return;
    }
    if (group.pending_ids.size === 0) {
      group.settled = true;
      group.source.socket.write(encodeIpcFrame(
        errorResponse(group.request.requestId, "no-client-found")
      ));
    }
  }
}

export interface FakeCodexDesktopOwnerOptions {
  endpoint: CodexIpcEndpoint;
  conversation_id: string;
  start_turn?: (request: CodexDesktopStartTurnRequest) => unknown | Promise<unknown>;
}

export class FakeCodexDesktopOwner {
  readonly #socket: Socket;
  readonly #conversationId: string;
  readonly #startTurn: (request: CodexDesktopStartTurnRequest) => unknown | Promise<unknown>;
  readonly #decoder = new IpcFrameDecoder();
  #clientId = "initializing-client";
  #initializeRequestId = "";
  #initializeResolve: (() => void) | undefined;
  #initializeReject: ((error: Error) => void) | undefined;
  last_start_turn: CodexDesktopStartTurnRequest | undefined;
  last_thread_settings: CodexDesktopUpdateThreadSettingsRequest | undefined;

  private constructor(socket: Socket, options: FakeCodexDesktopOwnerOptions) {
    this.#socket = socket;
    this.#conversationId = options.conversation_id;
    this.#startTurn = options.start_turn ?? (() => ({ ok: true }));
    socket.on("data", (chunk: Buffer) => this.#handleChunk(chunk));
    socket.on("error", (error) => this.#initializeReject?.(error));
    socket.on("close", () => this.#initializeReject?.(new Error("fake Codex owner closed")));
  }

  static async connect(options: FakeCodexDesktopOwnerOptions): Promise<FakeCodexDesktopOwner> {
    const socket = await connectEndpoint(options.endpoint);
    const owner = new FakeCodexDesktopOwner(socket, options);
    await owner.#initialize();
    return owner;
  }

  close(): void {
    this.#socket.destroy();
  }

  #initialize(): Promise<void> {
    this.#initializeRequestId = randomUUID();
    const initialized = new Promise<void>((resolve, reject) => {
      this.#initializeResolve = resolve;
      this.#initializeReject = reject;
    });
    this.#socket.write(encodeIpcFrame({
      type: "request",
      requestId: this.#initializeRequestId,
      sourceClientId: this.#clientId,
      version: 0,
      method: "initialize",
      params: { clientType: "tokn-codex-lab-owner" }
    }));
    return initialized;
  }

  #handleChunk(chunk: Buffer): void {
    try {
      for (const message of this.#decoder.push(chunk)) {
        void this.#handleMessage(message);
      }
    } catch (error) {
      this.#socket.destroy(asError(error));
    }
  }

  async #handleMessage(message: unknown): Promise<void> {
    if (!isRecord(message) || typeof message.type !== "string") {
      return;
    }
    if (
      message.type === "response"
      && message.requestId === this.#initializeRequestId
      && message.resultType === "success"
      && isRecord(message.result)
      && typeof message.result.clientId === "string"
    ) {
      this.#clientId = message.result.clientId;
      this.#initializeResolve?.();
      this.#initializeResolve = undefined;
      this.#initializeReject = undefined;
      return;
    }
    if (
      message.type === "client-discovery-request"
      && typeof message.requestId === "string"
    ) {
      const response: CodexDesktopClientDiscoveryResponse = {
        type: "client-discovery-response",
        requestId: message.requestId,
        response: {
          canHandle: isRoutableThreadRequest(message.request)
            && message.request.params.conversationId === this.#conversationId
        }
      };
      this.#socket.write(encodeIpcFrame(response));
      return;
    }
    if (isUpdateThreadSettingsRequest(message)) {
      this.last_thread_settings = message;
      this.#socket.write(encodeIpcFrame({
        type: "response",
        requestId: message.requestId,
        resultType: "success",
        method: message.method,
        handledByClientId: this.#clientId,
        result: { ok: true }
      }));
      return;
    }
    if (!isStartTurnRequest(message)) {
      return;
    }
    this.last_start_turn = message;
    try {
      const result = await this.#startTurn(message);
      this.#socket.write(encodeIpcFrame({
        type: "response",
        requestId: message.requestId,
        resultType: "success",
        method: message.method,
        handledByClientId: this.#clientId,
        result: { result }
      }));
    } catch (error) {
      this.#socket.write(encodeIpcFrame(errorResponse(
        message.requestId,
        asError(error).message
      )));
    }
  }
}

function isRoutableThreadRequest(
  value: unknown
): value is CodexDesktopStartTurnRequest | CodexDesktopUpdateThreadSettingsRequest {
  return isStartTurnRequest(value) || isUpdateThreadSettingsRequest(value);
}

function isStartTurnRequest(value: unknown): value is CodexDesktopStartTurnRequest {
  return isRecord(value)
    && value.type === "request"
    && value.method === "thread-follower-start-turn"
    && value.version === CODEX_DESKTOP_START_TURN_VERSION
    && typeof value.requestId === "string"
    && isRecord(value.params)
    && typeof value.params.conversationId === "string"
    && isRecord(value.params.turnStartParams)
    && Array.isArray(value.params.turnStartParams.input);
}

function isUpdateThreadSettingsRequest(
  value: unknown
): value is CodexDesktopUpdateThreadSettingsRequest {
  return isRecord(value)
    && value.type === "request"
    && value.method === "thread-follower-update-thread-settings"
    && value.version === 1
    && typeof value.requestId === "string"
    && isRecord(value.params)
    && typeof value.params.conversationId === "string"
    && isRecord(value.params.threadSettings);
}

function errorResponse(requestId: string, error: string): Record<string, unknown> {
  return {
    type: "response",
    requestId,
    resultType: "error",
    error
  };
}

function listen(server: Server, path: string): Promise<void> {
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(path, () => {
      server.off("error", reject);
      resolve();
    });
  });
}

function close(server: Server): Promise<void> {
  return new Promise((resolve, reject) => {
    server.close((error) => error ? reject(error) : resolve());
  });
}

function connectEndpoint(endpoint: CodexIpcEndpoint): Promise<Socket> {
  return new Promise((resolve, reject) => {
    const socket = createConnection(ipcEndpointAddress(endpoint));
    socket.once("connect", () => {
      socket.off("error", reject);
      resolve(socket);
    });
    socket.once("error", reject);
  });
}

async function prepareEndpoint(endpoint: CodexIpcEndpoint): Promise<void> {
  if (endpoint.transport === "windows_pipe") {
    return;
  }
  try {
    const metadata = await lstat(endpoint.path);
    throw new Error(
      `refusing to replace existing ${metadata.isSocket() ? "socket" : "path"}: ${endpoint.path}`
    );
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
      throw error;
    }
  }
}

async function secureEndpoint(endpoint: CodexIpcEndpoint): Promise<void> {
  if (endpoint.transport === "unix_socket") {
    await chmod(endpoint.path, 0o600);
  }
}

async function removeEndpoint(endpoint: CodexIpcEndpoint): Promise<void> {
  if (endpoint.transport === "windows_pipe") {
    return;
  }
  await unlink(endpoint.path).catch((error: NodeJS.ErrnoException) => {
    if (error.code !== "ENOENT") {
      throw error;
    }
  });
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
