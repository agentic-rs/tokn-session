import { statSync } from "node:fs";
import { homedir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";

import {
  CodexDesktopInputClient,
  type CodexDesktopInputAdmission,
} from "../../../extensions/codex/lib/desktop-input-client";
import {
  codexDesktopIpcEndpoint,
  type CodexIpcEndpoint,
} from "../../../extensions/codex/lib/ipc-endpoint";
import type { RelayEvent } from "./protocol";

const MAX_INPUT_LENGTH = 16 * 1024;
const MAX_CLI_ERROR_BYTES = 32 * 1024;

export type CodexInputStrategy = "auto" | "ipc" | "cli";
export type CodexInputRoute = "ipc" | "cli";

export interface CodexInputOverrides {
  model?: string;
  effort?: string;
}

export interface CodexSessionTarget {
  provider: "codex";
  path: string;
  session_id: string;
  cwd?: string;
  parent_session_id?: string | null;
  agent_path?: string | null;
}

export interface CodexInputAdmission {
  provider: "codex";
  route: CodexInputRoute;
  session_id: string;
}

export function codexInputAdmissionStatus(
  admission: CodexInputAdmission
): string {
  return admission.route === "ipc"
    ? "Codex App accepted input"
    : "Codex CLI completed input";
}

export type CodexIpcSubmitter = (
  target: CodexSessionTarget,
  prompt: string,
  overrides: CodexInputOverrides
) => Promise<CodexDesktopInputAdmission>;

export type CodexCliResumer = (
  target: CodexSessionTarget,
  prompt: string,
  overrides: CodexInputOverrides
) => Promise<void>;

export interface CodexInputBrokerOptions {
  strategy?: CodexInputStrategy;
  endpoint?: CodexIpcEndpoint;
  codex_bin?: string;
  ipc_submit?: CodexIpcSubmitter;
  cli_resume?: CodexCliResumer;
  read_rollout_settings?: (path: string) => Promise<CodexInputOverrides>;
}

export class CodexInputBroker {
  readonly #strategy: CodexInputStrategy;
  readonly #ipcSubmit: CodexIpcSubmitter;
  readonly #cliResume: CodexCliResumer;
  readonly #readRolloutSettings: (path: string) => Promise<CodexInputOverrides>;
  readonly #targets = new Map<string, CodexSessionTarget>();
  readonly #inFlight = new Map<string, Promise<CodexInputAdmission>>();

  constructor(options: CodexInputBrokerOptions = {}) {
    const endpoint = options.endpoint
      ?? codexDesktopIpcEndpoint(
        process.env.CODEX_HOME ?? join(homedir(), ".codex")
      );
    const codexBin = normalizeCodexBin(
      options.codex_bin ?? process.env.TOKN_CODEX_BIN ?? "codex"
    );
    this.#strategy = options.strategy ?? "auto";
    this.#ipcSubmit = options.ipc_submit
      ?? ((target, prompt, overrides) => (
        submitCodexIpc(endpoint, target, prompt, overrides)
      ));
    this.#cliResume = options.cli_resume
      ?? ((target, prompt, overrides) => (
        resumeCodexCli(codexBin, target, prompt, overrides)
      ));
    this.#readRolloutSettings = options.read_rollout_settings ?? readRolloutSettings;
  }

  observe(event: RelayEvent): void {
    const separator = event.topic.indexOf(".");
    const provider = event.session.provider
      ?? (separator > 0 ? event.topic.slice(0, separator) : undefined);
    if (provider?.toLowerCase() !== "codex" || !event.path) {
      return;
    }
    this.#targets.set(event.topic, {
      provider: "codex",
      path: event.path,
      session_id: event.session.session_id,
      cwd: event.session.cwd ?? undefined,
      parent_session_id: event.session.parent_session_id,
      agent_path: event.session.agent_path
    });
  }

  async submit(
    topic: string,
    prompt: string,
    overrides: CodexInputOverrides = {}
  ): Promise<CodexInputAdmission> {
    const target = this.#targets.get(topic);
    if (!target) {
      throw new Error("input is only available for an observed Codex session");
    }
    assertRootSession(target);
    const normalizedPrompt = normalizePrompt(prompt);
    const normalizedOverrides = normalizeOverrides(overrides);
    const sessionPath = existingRolloutPath(target.path);
    if (this.#inFlight.has(sessionPath)) {
      throw new Error("that Codex session already has input in flight");
    }

    const run = this.#submit(target, normalizedPrompt, normalizedOverrides);
    this.#inFlight.set(sessionPath, run);
    try {
      return await run;
    } finally {
      if (this.#inFlight.get(sessionPath) === run) {
        this.#inFlight.delete(sessionPath);
      }
    }
  }

  async #submit(
    target: CodexSessionTarget,
    prompt: string,
    overrides: CodexInputOverrides
  ): Promise<CodexInputAdmission> {
    if (this.#strategy === "cli") {
      return this.#submitCli(target, prompt, overrides);
    }
    try {
      await this.#ipcSubmit(target, prompt, overrides);
      return {
        provider: "codex",
        route: "ipc",
        session_id: target.session_id
      };
    } catch (error) {
      if (this.#strategy === "ipc" || !isSafeCliFallback(error)) {
        throw error;
      }
      return this.#submitCli(target, prompt, overrides);
    }
  }

  async #submitCli(
    target: CodexSessionTarget,
    prompt: string,
    overrides: CodexInputOverrides
  ): Promise<CodexInputAdmission> {
    assertCliTarget(target);
    const retainedSettings = await this.#readRolloutSettings(target.path);
    await this.#cliResume(target, prompt, {
      ...retainedSettings,
      ...overrides
    });
    return {
      provider: "codex",
      route: "cli",
      session_id: target.session_id
    };
  }
}

async function submitCodexIpc(
  endpoint: CodexIpcEndpoint,
  target: CodexSessionTarget,
  prompt: string,
  overrides: CodexInputOverrides
): Promise<CodexDesktopInputAdmission> {
  const client = await CodexDesktopInputClient.connect({ endpoint });
  try {
    return await client.startTurn(target.session_id, prompt, overrides);
  } finally {
    client.close();
  }
}

export function codexCliResumeCommand(
  codexBin: string,
  target: CodexSessionTarget,
  overrides: CodexInputOverrides
): string[] {
  const command = [
    normalizeCodexBin(codexBin),
    "exec",
    "resume",
    "--json",
    "--skip-git-repo-check"
  ];
  if (overrides.model) {
    command.push("--model", overrides.model);
  }
  if (overrides.effort) {
    command.push("-c", `model_reasoning_effort=${JSON.stringify(overrides.effort)}`);
  }
  command.push(target.session_id, "-");
  return command;
}

async function resumeCodexCli(
  codexBin: string,
  target: CodexSessionTarget,
  prompt: string,
  overrides: CodexInputOverrides
): Promise<void> {
  const child = Bun.spawn(codexCliResumeCommand(codexBin, target, overrides), {
    cwd: existingWorkingDirectory(target.cwd),
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe"
  });
  child.stdin.write(prompt);
  child.stdin.end();
  const stderrPromise = drainStream(child.stderr, MAX_CLI_ERROR_BYTES);
  const stdoutPromise = drainStream(child.stdout, 0);
  const [exitCode, stderr] = await Promise.all([
    child.exited,
    stderrPromise,
    stdoutPromise
  ]).then(([code, errorText]) => [code, errorText] as const);
  if (exitCode !== 0) {
    const detail = stderr.trim();
    throw new Error(
      detail
        ? `Codex CLI resume failed: ${detail}`
        : `Codex CLI resume exited with status ${exitCode}`
    );
  }
}

async function drainStream(
  stream: ReadableStream<Uint8Array>,
  captureBytes: number
): Promise<string> {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let captured = "";
  while (true) {
    const result = await reader.read();
    if (result.done) {
      break;
    }
    if (captureBytes > 0 && Buffer.byteLength(captured, "utf8") < captureBytes) {
      captured += decoder.decode(result.value, { stream: true });
      if (Buffer.byteLength(captured, "utf8") > captureBytes) {
        captured = Buffer.from(captured, "utf8")
          .subarray(0, captureBytes)
          .toString("utf8");
      }
    }
  }
  return captured + decoder.decode();
}

function normalizePrompt(prompt: string): string {
  const normalized = prompt.trim();
  if (!normalized) {
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

function normalizeOverrides(overrides: CodexInputOverrides): CodexInputOverrides {
  return {
    ...normalizeOverride("model", overrides.model),
    ...normalizeOverride("effort", overrides.effort)
  };
}

function normalizeOverride(
  field: "model" | "effort",
  value: string | undefined
): CodexInputOverrides {
  if (value === undefined) {
    return {};
  }
  const normalized = value.trim();
  if (!normalized) {
    throw new Error(`Codex ${field} override is empty`);
  }
  return { [field]: normalized };
}

export async function readRolloutSettings(
  path: string
): Promise<CodexInputOverrides> {
  const decoder = new TextDecoder();
  const reader = Bun.file(existingRolloutPath(path)).stream().getReader();
  let buffer = "";
  let settings: CodexInputOverrides = {};
  while (true) {
    const result = await reader.read();
    if (result.done) {
      break;
    }
    buffer += decoder.decode(result.value, { stream: true });
    let newline = buffer.indexOf("\n");
    while (newline >= 0) {
      settings = settingsFromLine(buffer.slice(0, newline), settings);
      buffer = buffer.slice(newline + 1);
      newline = buffer.indexOf("\n");
    }
  }
  buffer += decoder.decode();
  return settingsFromLine(buffer, settings);
}

function settingsFromLine(
  line: string,
  current: CodexInputOverrides
): CodexInputOverrides {
  if (!line.includes("thread_settings_applied") && !line.includes("turn_context")) {
    return current;
  }
  try {
    const record = JSON.parse(line) as {
      type?: unknown;
      payload?: {
        type?: unknown;
        thread_settings?: {
          model?: unknown;
          reasoning_effort?: unknown;
        };
        model?: unknown;
        effort?: unknown;
      };
    };
    if (record.type === "turn_context") {
      return settingsFromValues(record.payload?.model, record.payload?.effort, current);
    }
    if (
      record.type !== "event_msg"
      || record.payload?.type !== "thread_settings_applied"
    ) {
      return current;
    }
    const native = record.payload.thread_settings;
    return settingsFromValues(native?.model, native?.reasoning_effort, current);
  } catch {
    return current;
  }
}

function settingsFromValues(
  model: unknown,
  effort: unknown,
  current: CodexInputOverrides
): CodexInputOverrides {
  return {
    ...(typeof model === "string" && model.trim()
      ? { model: model.trim() }
      : current.model ? { model: current.model } : {}),
    ...(typeof effort === "string" && effort.trim()
      ? { effort: effort.trim() }
      : current.effort ? { effort: current.effort } : {})
  };
}

function normalizeCodexBin(value: string): string {
  const normalized = value.trim();
  if (!normalized) {
    throw new Error("Codex CLI executable is empty");
  }
  return normalized;
}

function existingRolloutPath(value: string): string {
  if (!isAbsolute(value)) {
    throw new Error("the observed Codex rollout path must be absolute");
  }
  const candidate = resolve(value);
  try {
    if (!statSync(candidate).isFile()) {
      throw new Error("not a regular file");
    }
    return candidate;
  } catch {
    throw new Error("the observed Codex rollout file is unavailable");
  }
}

function existingWorkingDirectory(value: string | undefined): string {
  if (!value || !isAbsolute(value)) {
    throw new Error("Codex CLI fallback requires an observed absolute session cwd");
  }
  const candidate = resolve(value);
  try {
    if (!statSync(candidate).isDirectory()) {
      throw new Error("not a directory");
    }
    return candidate;
  } catch {
    throw new Error("the observed Codex session cwd is unavailable");
  }
}

function assertCliTarget(target: CodexSessionTarget): void {
  existingWorkingDirectory(target.cwd);
}

function assertRootSession(target: CodexSessionTarget): void {
  if (
    target.parent_session_id
    || (target.agent_path && target.agent_path !== "/root")
  ) {
    throw new Error("Codex input is only available for root sessions");
  }
}

function isSafeCliFallback(error: unknown): boolean {
  const code = typeof error === "object" && error !== null && "code" in error
    ? String(error.code)
    : undefined;
  if (code === "ENOENT" || code === "ECONNREFUSED") {
    return true;
  }
  const message = error instanceof Error ? error.message : String(error);
  return /^Codex desktop IPC thread-follower-(?:start-turn|update-thread-settings) failed: no-client-found(?:$|:)/u
    .test(message);
}
