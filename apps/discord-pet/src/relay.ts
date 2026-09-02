import { resolve } from "node:path";

import { consumeJsonl, JsonlDecoder } from "./jsonl";
import { parseRelayRecord, RelayActivityDispatcher, type RelayEvent } from "./protocol";

export interface RelayOptions {
  stdin: boolean;
  relay_bin?: string;
  codex_dir?: string;
  pi_dir?: string;
  signal?: AbortSignal;
  diagnostics?: "inherit" | "discard";
}

interface RelayChild {
  process: Bun.Subprocess<"ignore", "pipe", "pipe">;
  stream: ReadableStream<Uint8Array>;
}

export async function followRelay(
  options: RelayOptions,
  onEvent: (event: RelayEvent) => Promise<void>
): Promise<void> {
  const abort = new AbortController();
  const child = options.stdin ? undefined : spawnRelay(options);
  const stream = child?.stream ?? Bun.stdin.stream();
  const stop = (): void => {
    abort.abort();
    if (child?.process.exitCode === null) {
      child.process.kill("SIGTERM");
    }
  };
  process.once("SIGINT", stop);
  process.once("SIGTERM", stop);
  process.once("SIGHUP", stop);
  options.signal?.addEventListener("abort", stop, { once: true });
  if (options.signal?.aborted) {
    stop();
  }

  try {
    const diagnostics = child
      ? copyDiagnostics(
        child.process.stderr,
        abort.signal,
        options.diagnostics ?? "inherit"
      )
      : Promise.resolve();
    const decoder = new JsonlDecoder(parseRelayRecord);
    const activity = new RelayActivityDispatcher();
    await consumeJsonl(
      stream,
      decoder,
      (record) => activity.dispatch(record, onEvent, abort.signal),
      abort.signal
    );
    if (child) {
      const exitCode = await child.process.exited;
      await diagnostics;
      if (!abort.signal.aborted && exitCode !== 0) {
        throw new Error(`Relay exited with status ${exitCode}`);
      }
    }
    if (decoder.stats.malformed_lines > 0 || decoder.stats.oversized_lines > 0) {
      process.stderr.write(
        `Relay ignored ${decoder.stats.malformed_lines} malformed and `
        + `${decoder.stats.oversized_lines} oversized JSONL lines\n`
      );
    }
  } finally {
    stop();
    process.off("SIGINT", stop);
    process.off("SIGTERM", stop);
    process.off("SIGHUP", stop);
    options.signal?.removeEventListener("abort", stop);
    if (child?.process.exitCode === null) {
      await Promise.race([child.process.exited, Bun.sleep(1_000)]);
      if (child.process.exitCode === null) {
        child.process.kill("SIGKILL");
        await child.process.exited;
      }
    }
  }
}

function spawnRelay(options: RelayOptions): RelayChild {
  const workspaceRoot = resolve(import.meta.dir, "..", "..", "..");
  const workspaceBinary = resolve(
    workspaceRoot,
    "target",
    "debug",
    process.platform === "win32"
      ? "tokn-session-relay.exe"
      : "tokn-session-relay"
  );
  if (!options.relay_bin) {
    const build = Bun.spawnSync({
      cmd: ["cargo", "build", "-q", "-p", "tokn-session-relay"],
      cwd: workspaceRoot,
      stdout: "pipe",
      stderr: "pipe"
    });
    if (build.exitCode !== 0) {
      const diagnostic = build.stderr.toString().trim();
      throw new Error(diagnostic || `cargo build exited with status ${build.exitCode}`);
    }
  }

  const command = [
    options.relay_bin ?? workspaceBinary,
    "stdout",
    "--format",
    "json"
  ];
  if (options.codex_dir) {
    command.push("--codex-dir", options.codex_dir);
  }
  if (options.pi_dir) {
    command.push("--pi-dir", options.pi_dir);
  }
  const child = Bun.spawn({
    cmd: command,
    cwd: workspaceRoot,
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe"
  });
  return {
    process: child,
    stream: child.stdout
  };
}

async function copyDiagnostics(
  stream: ReadableStream<Uint8Array>,
  signal: AbortSignal,
  mode: "inherit" | "discard"
): Promise<void> {
  const reader = stream.getReader();
  const cancel = (): void => {
    void reader.cancel("Relay diagnostic consumption aborted").catch(() => {});
  };
  signal.addEventListener("abort", cancel, { once: true });
  try {
    while (!signal.aborted) {
      const result = await reader.read();
      if (result.done) {
        return;
      }
      if (mode === "inherit") {
        process.stderr.write(result.value);
      }
    }
  } finally {
    signal.removeEventListener("abort", cancel);
    reader.releaseLock();
  }
}
