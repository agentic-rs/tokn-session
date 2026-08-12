/** Private Codex App IPC contract observed in desktop build 26.727.51351. */
export const CODEX_DESKTOP_START_TURN_VERSION = 1;
export const CODEX_DESKTOP_INITIALIZE_VERSION = 0;
export const CODEX_DESKTOP_MAX_FRAME_BYTES = 1024 * 1024;

export interface CodexDesktopTextInput {
  type: "text";
  text: string;
}

export interface CodexDesktopTurnStartParams {
  input: CodexDesktopTextInput[];
  clientUserMessageId: string;
  additionalContext: null;
}

export interface CodexDesktopThreadSettings {
  model?: string;
  effort?: string;
}

export interface CodexDesktopInitializeRequest {
  type: "request";
  requestId: string;
  sourceClientId: string;
  version: typeof CODEX_DESKTOP_INITIALIZE_VERSION;
  method: "initialize";
  params: {
    clientType: string;
  };
}

export interface CodexDesktopStartTurnRequest {
  type: "request";
  requestId: string;
  sourceClientId: string;
  version: typeof CODEX_DESKTOP_START_TURN_VERSION;
  method: "thread-follower-start-turn";
  params: {
    conversationId: string;
    turnStartParams: CodexDesktopTurnStartParams;
  };
  timeoutMs: number;
}

export interface CodexDesktopUpdateThreadSettingsRequest {
  type: "request";
  requestId: string;
  sourceClientId: string;
  version: 1;
  method: "thread-follower-update-thread-settings";
  params: {
    conversationId: string;
    threadSettings: CodexDesktopThreadSettings;
  };
  timeoutMs: number;
}

export type CodexDesktopRequest =
  | CodexDesktopInitializeRequest
  | CodexDesktopStartTurnRequest
  | CodexDesktopUpdateThreadSettingsRequest;

export interface CodexDesktopSuccessResponse {
  type: "response";
  requestId: string;
  resultType: "success";
  method: string;
  handledByClientId: string;
  result: unknown;
}

export interface CodexDesktopErrorResponse {
  type: "response";
  requestId: string;
  resultType: "error";
  error: string;
}

export type CodexDesktopResponse =
  | CodexDesktopSuccessResponse
  | CodexDesktopErrorResponse;

export interface CodexDesktopClientDiscoveryRequest {
  type: "client-discovery-request";
  requestId: string;
  request: CodexDesktopRequest;
}

export interface CodexDesktopClientDiscoveryResponse {
  type: "client-discovery-response";
  requestId: string;
  response: {
    canHandle: boolean;
  };
}

export type CodexDesktopIpcMessage =
  | CodexDesktopRequest
  | CodexDesktopResponse
  | CodexDesktopClientDiscoveryRequest
  | CodexDesktopClientDiscoveryResponse
  | Record<string, unknown>;

export function encodeIpcFrame(message: unknown): Buffer {
  const payload = Buffer.from(JSON.stringify(message), "utf8");
  if (payload.byteLength > CODEX_DESKTOP_MAX_FRAME_BYTES) {
    throw new Error("Codex desktop IPC frame exceeds the experiment limit");
  }
  const header = Buffer.allocUnsafe(4);
  header.writeUInt32LE(payload.byteLength, 0);
  return Buffer.concat([header, payload]);
}

export class IpcFrameDecoder {
  #buffer = Buffer.alloc(0);

  push(chunk: Uint8Array): unknown[] {
    this.#buffer = Buffer.concat([this.#buffer, Buffer.from(chunk)]);
    const messages: unknown[] = [];

    while (this.#buffer.byteLength >= 4) {
      const length = this.#buffer.readUInt32LE(0);
      if (length > CODEX_DESKTOP_MAX_FRAME_BYTES) {
        throw new Error("Codex desktop IPC frame exceeds the experiment limit");
      }
      if (this.#buffer.byteLength < 4 + length) {
        break;
      }
      const payload = this.#buffer.subarray(4, 4 + length);
      this.#buffer = this.#buffer.subarray(4 + length);
      messages.push(JSON.parse(payload.toString("utf8")) as unknown);
    }

    return messages;
  }
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
