import { randomUUID } from "node:crypto";
import { createConnection, type Socket } from "node:net";

import {
  CODEX_DESKTOP_INITIALIZE_VERSION,
  CODEX_DESKTOP_START_TURN_VERSION,
  encodeIpcFrame,
  IpcFrameDecoder,
  isRecord,
  type CodexDesktopClientDiscoveryResponse,
  type CodexDesktopErrorResponse,
  type CodexDesktopRequest,
  type CodexDesktopResponse,
  type CodexDesktopStartTurnRequest,
  type CodexDesktopSuccessResponse,
} from "./ipc-protocol";

const DEFAULT_TIMEOUT_MS = 5_000;
const INITIAL_CLIENT_ID = "initializing-client";

interface PendingRequest {
  resolve: (response: CodexDesktopResponse) => void;
  reject: (error: Error) => void;
  timeout: ReturnType<typeof setTimeout>;
}

export interface CodexDesktopInputClientOptions {
  socket_path: string;
  client_type?: string;
  timeout_ms?: number;
}

export interface CodexDesktopInputAdmission {
  conversation_id: string;
  request_id: string;
  handled_by_client_id: string;
  result: unknown;
}

export class CodexDesktopInputClient {
  readonly #socket: Socket;
  readonly #clientType: string;
  readonly #timeoutMs: number;
  readonly #decoder = new IpcFrameDecoder();
  readonly #pending = new Map<string, PendingRequest>();
  #clientId = INITIAL_CLIENT_ID;
  #closed = false;

  private constructor(socket: Socket, options: CodexDesktopInputClientOptions) {
    this.#socket = socket;
    this.#clientType = options.client_type ?? "tokn-terminal-pet";
    this.#timeoutMs = options.timeout_ms ?? DEFAULT_TIMEOUT_MS;
    socket.on("data", (chunk: Buffer) => this.#handleChunk(chunk));
    socket.on("error", (error) => this.#failPending(error));
    socket.on("close", () => this.#failPending(new Error("Codex desktop IPC connection closed")));
  }

  static async connect(
    options: CodexDesktopInputClientOptions
  ): Promise<CodexDesktopInputClient> {
    if (!options.socket_path) {
      throw new Error("an explicit Codex desktop IPC socket path is required");
    }
    const socket = await connectSocket(options.socket_path, options.timeout_ms ?? DEFAULT_TIMEOUT_MS);
    const client = new CodexDesktopInputClient(socket, options);
    await client.#initialize();
    return client;
  }

  async startTurn(
    conversationId: string,
    prompt: string
  ): Promise<CodexDesktopInputAdmission> {
    const normalizedConversationId = conversationId.trim();
    const normalizedPrompt = prompt.trim();
    if (!normalizedConversationId) {
      throw new Error("Codex conversation id is required");
    }
    if (!normalizedPrompt) {
      throw new Error("Codex input message is empty");
    }

    const requestId = randomUUID();
    const request: CodexDesktopStartTurnRequest = {
      type: "request",
      requestId,
      sourceClientId: this.#clientId,
      version: CODEX_DESKTOP_START_TURN_VERSION,
      method: "thread-follower-start-turn",
      params: {
        conversationId: normalizedConversationId,
        turnStartParams: {
          input: [{ type: "text", text: normalizedPrompt }],
          clientUserMessageId: randomUUID(),
          additionalContext: null
        }
      },
      timeoutMs: this.#timeoutMs
    };
    const response = await this.#request(request);
    const success = expectSuccess(response, request.method);
    return {
      conversation_id: normalizedConversationId,
      request_id: requestId,
      handled_by_client_id: success.handledByClientId,
      result: success.result
    };
  }

  close(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#socket.destroy();
    this.#failPending(new Error("Codex desktop IPC client closed"));
  }

  async #initialize(): Promise<void> {
    const response = await this.#request({
      type: "request",
      requestId: randomUUID(),
      sourceClientId: INITIAL_CLIENT_ID,
      version: CODEX_DESKTOP_INITIALIZE_VERSION,
      method: "initialize",
      params: { clientType: this.#clientType }
    });
    const success = expectSuccess(response, "initialize");
    if (!isRecord(success.result) || typeof success.result.clientId !== "string") {
      throw new Error("Codex desktop IPC initialize response omitted clientId");
    }
    this.#clientId = success.result.clientId;
  }

  #request(request: CodexDesktopRequest): Promise<CodexDesktopResponse> {
    if (this.#closed || !this.#socket.writable) {
      return Promise.reject(new Error("Codex desktop IPC connection is unavailable"));
    }
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.#pending.delete(request.requestId);
        reject(new Error(`Codex desktop IPC request timed out: ${String(request.method)}`));
      }, this.#timeoutMs);
      this.#pending.set(request.requestId, { resolve, reject, timeout });
      this.#socket.write(encodeIpcFrame(request), (error) => {
        if (!error) {
          return;
        }
        const pending = this.#pending.get(request.requestId);
        if (pending) {
          clearTimeout(pending.timeout);
          this.#pending.delete(request.requestId);
          pending.reject(error);
        }
      });
    });
  }

  #handleChunk(chunk: Buffer): void {
    let messages: unknown[];
    try {
      messages = this.#decoder.push(chunk);
    } catch (error) {
      this.#socket.destroy();
      this.#failPending(asError(error));
      return;
    }
    for (const message of messages) {
      this.#handleMessage(message);
    }
  }

  #handleMessage(message: unknown): void {
    if (!isRecord(message) || typeof message.type !== "string") {
      return;
    }
    if (message.type === "client-discovery-request" && typeof message.requestId === "string") {
      const response: CodexDesktopClientDiscoveryResponse = {
        type: "client-discovery-response",
        requestId: message.requestId,
        response: { canHandle: false }
      };
      this.#socket.write(encodeIpcFrame(response));
      return;
    }
    if (message.type !== "response" || typeof message.requestId !== "string") {
      return;
    }
    const pending = this.#pending.get(message.requestId);
    if (!pending) {
      return;
    }
    clearTimeout(pending.timeout);
    this.#pending.delete(message.requestId);
    if (message.resultType === "error" && typeof message.error === "string") {
      pending.resolve(message as unknown as CodexDesktopErrorResponse);
      return;
    }
    if (
      message.resultType === "success"
      && typeof message.method === "string"
      && typeof message.handledByClientId === "string"
    ) {
      pending.resolve(message as unknown as CodexDesktopSuccessResponse);
      return;
    }
    pending.reject(new Error("Codex desktop IPC returned an invalid response"));
  }

  #failPending(error: Error): void {
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(error);
    }
    this.#pending.clear();
  }
}

function expectSuccess(
  response: CodexDesktopResponse,
  method: string
): CodexDesktopSuccessResponse {
  if (response.resultType === "error") {
    throw new Error(`Codex desktop IPC ${method} failed: ${response.error}`);
  }
  if (response.method !== method) {
    throw new Error(`Codex desktop IPC response method mismatch for ${method}`);
  }
  return response;
}

function connectSocket(path: string, timeoutMs: number): Promise<Socket> {
  return new Promise((resolve, reject) => {
    const socket = createConnection(path);
    const timeout = setTimeout(() => {
      socket.destroy();
      reject(new Error("Codex desktop IPC connection timed out"));
    }, timeoutMs);
    socket.once("connect", () => {
      clearTimeout(timeout);
      socket.off("error", onError);
      resolve(socket);
    });
    const onError = (error: Error): void => {
      clearTimeout(timeout);
      reject(error);
    };
    socket.once("error", onError);
  });
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
