import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { createInterface } from "node:readline";

import { isRecord } from "../lib/ipc-protocol";

interface PendingRequest {
  resolve: (result: unknown) => void;
  reject: (error: Error) => void;
}

interface TurnState {
  text: string;
  completed?: AppServerTurnResult;
  resolve?: (result: AppServerTurnResult) => void;
  reject?: (error: Error) => void;
}

export interface AppServerLabOptions {
  codex_home: string;
  cwd: string;
  codex_bin?: string;
  model?: string;
  base_url?: string;
}

export interface AppServerTurnResult {
  turn_id: string;
  status: string;
  text: string;
  error: unknown;
}

export interface AppServerTurnHandle {
  turn_id: string;
  response: unknown;
  completion: Promise<AppServerTurnResult>;
}

export class IsolatedCodexAppServer {
  readonly #process: ChildProcessWithoutNullStreams;
  readonly #cwd: string;
  readonly #model: string;
  readonly #pending = new Map<number, PendingRequest>();
  readonly #turns = new Map<string, TurnState>();
  #nextId = 1;
  #threadId = "";
  #closed = false;
  #stderr = "";

  private constructor(
    process: ChildProcessWithoutNullStreams,
    cwd: string,
    model: string
  ) {
    this.#process = process;
    this.#cwd = cwd;
    this.#model = model;

    const lines = createInterface({ input: process.stdout });
    lines.on("line", (line) => this.#handleLine(line));
    process.stderr.setEncoding("utf8");
    process.stderr.on("data", (chunk: string) => {
      this.#stderr = `${this.#stderr}${chunk}`.slice(-16 * 1024);
    });
    process.once("exit", (code, signal) => {
      const detail = this.#stderr.trim();
      const error = new Error(
        `isolated codex app-server exited (${signal ?? code ?? "unknown"})${detail ? `: ${detail}` : ""}`
      );
      this.#failPending(error);
      for (const turn of this.#turns.values()) {
        turn.reject?.(error);
      }
    });
  }

  static async start(options: AppServerLabOptions): Promise<IsolatedCodexAppServer> {
    await mkdir(options.codex_home, { recursive: true, mode: 0o700 });
    await mkdir(options.cwd, { recursive: true });
    const model = options.model ?? "deepseek-v4-flash";
    const baseUrl = options.base_url ?? "http://localhost:4141/v1";
    const process = spawn(options.codex_bin ?? "codex", [
      "app-server",
      "--stdio",
      "-c",
      `model_provider="tokn-lab"`,
      "-c",
      `model=${JSON.stringify(model)}`,
      "-c",
      `model_providers.tokn-lab.name="Tokn Lab"`,
      "-c",
      `model_providers.tokn-lab.base_url=${JSON.stringify(baseUrl)}`,
      "-c",
      `model_providers.tokn-lab.wire_api="responses"`,
      "-c",
      `model_providers.tokn-lab.requires_openai_auth=false`
    ], {
      cwd: options.cwd,
      env: {
        ...processEnv(),
        CODEX_HOME: options.codex_home
      },
      stdio: ["pipe", "pipe", "pipe"]
    });
    const client = new IsolatedCodexAppServer(process, options.cwd, model);
    await client.#initialize();
    await client.#startThread();
    return client;
  }

  get thread_id(): string {
    return this.#threadId;
  }

  async startTurn(prompt: string): Promise<AppServerTurnHandle> {
    if (!this.#threadId) {
      throw new Error("isolated app-server thread is unavailable");
    }
    const response = await this.#request("turn/start", {
      threadId: this.#threadId,
      input: [{ type: "text", text: prompt }],
      cwd: this.#cwd,
      approvalPolicy: "never",
      sandboxPolicy: {
        type: "readOnly",
        access: { type: "fullAccess" }
      },
      model: this.#model
    });
    const turnId = nestedString(response, "turn", "id");
    if (!turnId) {
      throw new Error("turn/start response omitted turn.id");
    }
    const state = this.#turnState(turnId);
    const completion = state.completed
      ? Promise.resolve(state.completed)
      : new Promise<AppServerTurnResult>((resolve, reject) => {
          state.resolve = resolve;
          state.reject = reject;
        });
    return { turn_id: turnId, response, completion };
  }

  async close(): Promise<void> {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#process.stdin.end();
    if (this.#process.exitCode !== null || this.#process.signalCode !== null) {
      return;
    }
    await new Promise<void>((resolve) => {
      const timeout = setTimeout(() => {
        this.#process.kill("SIGTERM");
        resolve();
      }, 1_000);
      this.#process.once("exit", () => {
        clearTimeout(timeout);
        resolve();
      });
    });
  }

  async #initialize(): Promise<void> {
    await this.#request("initialize", {
      clientInfo: {
        name: "tokn_codex_input_lab",
        title: "Tokn Codex Input Lab",
        version: "0.1.0"
      }
    });
    this.#notify("initialized", {});
  }

  async #startThread(): Promise<void> {
    const response = await this.#request("thread/start", {
      model: this.#model,
      cwd: this.#cwd,
      approvalPolicy: "never",
      sandbox: "read-only",
      serviceName: "tokn_codex_input_lab"
    });
    const threadId = nestedString(response, "thread", "id");
    if (!threadId) {
      throw new Error("thread/start response omitted thread.id");
    }
    this.#threadId = threadId;
  }

  #request(method: string, params: unknown): Promise<unknown> {
    if (this.#closed || !this.#process.stdin.writable) {
      return Promise.reject(new Error("isolated codex app-server is unavailable"));
    }
    const id = this.#nextId++;
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject });
      this.#write({ method, id, params });
    });
  }

  #notify(method: string, params: unknown): void {
    this.#write({ method, params });
  }

  #write(message: unknown): void {
    this.#process.stdin.write(`${JSON.stringify(message)}\n`);
  }

  #handleLine(line: string): void {
    let message: unknown;
    try {
      message = JSON.parse(line) as unknown;
    } catch {
      return;
    }
    if (!isRecord(message)) {
      return;
    }
    if (typeof message.id === "number") {
      const pending = this.#pending.get(message.id);
      if (!pending) {
        return;
      }
      this.#pending.delete(message.id);
      if (isRecord(message.error)) {
        pending.reject(new Error(
          typeof message.error.message === "string"
            ? message.error.message
            : JSON.stringify(message.error)
        ));
      } else {
        pending.resolve(message.result);
      }
      return;
    }
    if (typeof message.method === "string" && isRecord(message.params)) {
      this.#handleNotification(message.method, message.params);
    }
  }

  #handleNotification(method: string, params: Record<string, unknown>): void {
    const turnId = nestedString(params, "turn", "id")
      ?? (typeof params.turnId === "string" ? params.turnId : undefined);
    if (!turnId) {
      return;
    }
    const state = this.#turnState(turnId);
    if (method === "item/agentMessage/delta" && typeof params.delta === "string") {
      state.text += params.delta;
      return;
    }
    if (method !== "turn/completed") {
      return;
    }
    const turn = isRecord(params.turn) ? params.turn : {};
    const result: AppServerTurnResult = {
      turn_id: turnId,
      status: typeof turn.status === "string" ? turn.status : "unknown",
      text: state.text,
      error: turn.error ?? null
    };
    state.completed = result;
    state.resolve?.(result);
    state.resolve = undefined;
    state.reject = undefined;
  }

  #turnState(turnId: string): TurnState {
    let state = this.#turns.get(turnId);
    if (!state) {
      state = { text: "" };
      this.#turns.set(turnId, state);
    }
    return state;
  }

  #failPending(error: Error): void {
    for (const pending of this.#pending.values()) {
      pending.reject(error);
    }
    this.#pending.clear();
  }
}

function nestedString(
  value: unknown,
  objectKey: string,
  valueKey: string
): string | undefined {
  if (!isRecord(value) || !isRecord(value[objectKey])) {
    return undefined;
  }
  const nested = value[objectKey];
  return typeof nested[valueKey] === "string" ? nested[valueKey] : undefined;
}

function processEnv(): NodeJS.ProcessEnv {
  return { ...process.env };
}
