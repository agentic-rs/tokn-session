#!/usr/bin/env bun

import { existsSync } from "node:fs";
import { resolve } from "node:path";

import {
  IMAGE_PROTOCOLS,
  PetImageController,
  resolveImageProtocol,
  type ImageProtocolOption
} from "./image_protocol";
import { loadPetArt, selectPose } from "./art";
import { consumeJsonl, JsonlDecoder } from "./jsonl";
import { parseRelayEvent, type RelayEvent } from "./protocol";
import { renderScreen, type RenderMeta } from "./renderer";
import { PET_STATES, PetStore, type PetSnapshot, type PetState } from "./state";
import { TerminalSurface } from "./terminal";

type RunMode = "relay" | "stdin" | "demo" | "snapshot";

interface Options {
  mode: RunMode;
  color: boolean;
  name: string;
  protocol: ImageProtocolOption;
  relay_bin?: string;
  snapshot_state?: PetState;
}

interface RelayChild {
  process: Bun.Subprocess<"ignore", "pipe", "pipe">;
  stream: ReadableStream<Uint8Array>;
  label: string;
}

const options = parseArgs(process.argv.slice(2));
const art = await loadPetArt();

if (options.mode === "snapshot") {
  writeSnapshot(options);
} else {
  await runInteractive(options);
}

async function runInteractive(runOptions: Options): Promise<void> {
  const store = new PetStore();
  const decoder = new JsonlDecoder(parseRelayEvent);
  const surface = new TerminalSurface();
  const imageProtocol = resolveImageProtocol(runOptions.protocol);
  const imageController = new PetImageController(art, imageProtocol);
  const meta: RenderMeta = {
    source_label: runOptions.mode === "demo"
      ? "demo"
      : runOptions.mode === "stdin"
        ? "stdin"
        : "relay"
  };
  const startedAt = Date.now();
  const sourceAbort = new AbortController();

  let child: RelayChild | undefined;
  let stopped = false;
  let exitCode = 0;
  let frameTimer: ReturnType<typeof setInterval> | undefined;
  let resolveStopped: (() => void) | undefined;
  const stoppedPromise = new Promise<void>((resolvePromise) => {
    resolveStopped = resolvePromise;
  });

  const stop = (code = 0, diagnostic?: string): void => {
    if (stopped) {
      return;
    }
    stopped = true;
    exitCode = code;
    if (diagnostic) {
      meta.diagnostic = diagnostic;
      render();
    }
    sourceAbort.abort();
    resolveStopped?.();
  };

  const render = (): void => {
    const nowMs = Date.now();
    const snapshot = runOptions.mode === "demo"
      ? demoSnapshot(startedAt, nowMs)
      : store.snapshot(nowMs);
    const pose = selectPose(snapshot.state, snapshot.state_changed_at, nowMs);
    const screen = renderScreen(snapshot, art[pose].ansi, {
      ...meta,
      stats: decoder.stats
    }, {
      columns: process.stdout.columns ?? 80,
      rows: process.stdout.rows ?? 24,
      color: runOptions.color,
      image_protocol: imageProtocol,
      name: runOptions.name
    });
    surface.render(screen.lines);
    if (screen.image_anchor) {
      process.stdout.write(imageController.draw(pose, screen.image_anchor));
    } else {
      process.stdout.write(imageController.clear());
    }
  };

  const onResize = (): void => {
    process.stdout.write(imageController.clear());
    surface.invalidate();
    render();
  };
  const onSignal = (): void => stop(0);
  const onKey = (chunk: Buffer): void => {
    for (const byte of chunk) {
      if (byte === 3 || byte === 27 || byte === 113) {
        stop(0);
      } else if (byte === 99) {
        const focus = store.snapshot().focus;
        store.acknowledge(focus?.topic);
        render();
      }
    }
  };

  let rawMode = false;
  try {
    surface.enter();
    render();
    frameTimer = setInterval(render, 120);
    process.stdout.on("resize", onResize);
    process.once("SIGINT", onSignal);
    process.once("SIGTERM", onSignal);
    process.once("SIGHUP", onSignal);

    if (runOptions.mode !== "stdin" && process.stdin.isTTY) {
      process.stdin.setRawMode(true);
      process.stdin.resume();
      process.stdin.on("data", onKey);
      rawMode = true;
    }

    if (runOptions.mode === "stdin") {
      void consumeJsonl(
        Bun.stdin.stream(),
        decoder,
        (event) => {
          store.ingest(event);
          render();
        },
        sourceAbort.signal
      )
        .then(() => stop(0))
        .catch((error: unknown) => stop(1, errorMessage(error)));
    } else if (runOptions.mode === "relay") {
      try {
        child = spawnRelay(runOptions);
        meta.source_label = child.label;
        void consumeRelayDiagnostics(child.process.stderr, sourceAbort.signal);
        void consumeJsonl(
          child.stream,
          decoder,
          (event) => {
            store.ingest(event);
            render();
          },
          sourceAbort.signal
        )
          .catch((error: unknown) => stop(1, errorMessage(error)));
        void child.process.exited.then((code) => {
          if (!stopped) {
            stop(code === 0 ? 0 : 1, code === 0 ? undefined : `Relay exited with status ${code}`);
          }
        });
      } catch (error) {
        stop(1, `Could not start Relay: ${errorMessage(error)}`);
      }
    }

    await stoppedPromise;
  } finally {
    sourceAbort.abort();
    if (frameTimer) {
      clearInterval(frameTimer);
    }
    process.stdout.off("resize", onResize);
    process.off("SIGINT", onSignal);
    process.off("SIGTERM", onSignal);
    process.off("SIGHUP", onSignal);
    process.stdin.off("data", onKey);
    if (rawMode) {
      process.stdin.setRawMode(false);
      process.stdin.pause();
    }
    process.stdout.write(imageController.clear());
    surface.leave();
    if (child && child.process.exitCode === null) {
      child.process.kill("SIGTERM");
      await Promise.race([
        child.process.exited,
        Bun.sleep(1_000)
      ]);
      if (child.process.exitCode === null) {
        child.process.kill("SIGKILL");
        await child.process.exited;
      }
    }
    process.exitCode = exitCode;
  }
}

function spawnRelay(runOptions: Options): RelayChild {
  const workspaceRoot = resolve(import.meta.dir, "..", "..", "..");
  const workspaceBinary = resolve(
    workspaceRoot,
    "target",
    "debug",
    process.platform === "win32" ? "tokn-session-relay.exe" : "tokn-session-relay"
  );
  if (!runOptions.relay_bin && !existsSync(workspaceBinary)) {
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

  const relayBinary = runOptions.relay_bin ?? workspaceBinary;
  const command = [relayBinary, "stdout", "--format", "json"];
  const childProcess = Bun.spawn({
    cmd: command,
    cwd: workspaceRoot,
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe"
  });
  return {
    process: childProcess,
    stream: childProcess.stdout,
    label: runOptions.relay_bin ? "relay binary" : "workspace relay"
  };
}

async function consumeRelayDiagnostics(
  stream: ReadableStream<Uint8Array>,
  signal: AbortSignal
): Promise<void> {
  const reader = stream.getReader();
  const cancel = (): void => {
    void reader.cancel("Relay diagnostic consumption aborted").catch(() => {});
  };
  signal.addEventListener("abort", cancel, { once: true });
  try {
    if (signal.aborted) {
      cancel();
    }
    while (true) {
      const result = await reader.read();
      if (result.done) {
        return;
      }
    }
  } finally {
    signal.removeEventListener("abort", cancel);
    reader.releaseLock();
  }
}

function writeSnapshot(runOptions: Options): void {
  const nowMs = Date.now();
  const state = runOptions.snapshot_state ?? "running";
  const snapshot = syntheticSnapshot(state, nowMs);
  const pose = selectPose(state, nowMs, nowMs);
  const screen = renderScreen(snapshot, art[pose].ansi, {
    source_label: "snapshot"
  }, {
    columns: 64,
    rows: 22,
    color: runOptions.color,
    image_protocol: "ansi",
    name: runOptions.name
  });
  const output = trimBlankLines(screen.lines).join("\n");
  process.stdout.write(`${output}\n`);
}

function demoSnapshot(startedAt: number, nowMs: number): PetSnapshot {
  const states: PetState[] = [
    "idle",
    "running",
    "needs_input",
    "ready",
    "blocked"
  ];
  const duration = 3_000;
  const index = Math.floor((nowMs - startedAt) / duration) % states.length;
  const state = states[index] ?? "idle";
  const stateChangedAt = startedAt + Math.floor((nowMs - startedAt) / duration) * duration;
  return syntheticSnapshot(state, stateChangedAt);
}

function syntheticSnapshot(state: PetState, stateChangedAt: number): PetSnapshot {
  return {
    state,
    state_changed_at: stateChangedAt,
    active_sessions: state === "idle" ? 0 : 1,
    total_sessions: 1,
    focus: {
      topic: "codex.demo",
      state,
      state_changed_at: stateChangedAt,
      last_event_at: stateChangedAt,
      label: demoLabel(state),
      provider: "codex",
      project: "tokn-agent",
      session_id: "demo",
      agent: "root"
    }
  };
}

function demoLabel(state: PetState): string {
  switch (state) {
    case "idle":
      return "Waiting for Relay activity";
    case "running":
      return "Editing the session renderer";
    case "needs_input":
      return "Approval required";
    case "ready":
      return "Task complete";
    case "blocked":
      return "Relay reported an error";
  }
}

function parseArgs(args: string[]): Options {
  const parsed: Options = {
    mode: "relay",
    color: !process.env.NO_COLOR && process.env.TERM !== "dumb",
    name: "Hachiware",
    protocol: "auto"
  };

  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]!;
    switch (argument) {
      case "--demo":
        parsed.mode = "demo";
        break;
      case "--stdin":
        parsed.mode = "stdin";
        break;
      case "--snapshot": {
        const value = args[++index];
        if (!value || !PET_STATES.includes(value as PetState)) {
          fail(`--snapshot expects one of: ${PET_STATES.join(", ")}`);
        }
        parsed.mode = "snapshot";
        parsed.snapshot_state = value as PetState;
        break;
      }
      case "--relay-bin":
        parsed.relay_bin = nextValue(args, ++index, "--relay-bin");
        break;
      case "--protocol": {
        const value = nextValue(args, ++index, "--protocol");
        if (!IMAGE_PROTOCOLS.includes(value as ImageProtocolOption)) {
          fail(`--protocol expects one of: ${IMAGE_PROTOCOLS.join(", ")}`);
        }
        parsed.protocol = value as ImageProtocolOption;
        break;
      }
      case "--name":
        parsed.name = nextValue(args, ++index, "--name");
        break;
      case "--no-color":
        parsed.color = false;
        break;
      case "--help":
      case "-h":
        process.stdout.write(`${help()}\n`);
        process.exit(0);
      default:
        fail(`unknown option: ${argument}`);
    }
  }
  return parsed;
}

function nextValue(args: string[], index: number, option: string): string {
  const value = args[index];
  if (!value) {
    fail(`${option} requires a value`);
  }
  return value;
}

function fail(message: string): never {
  process.stderr.write(`${message}\n\n${help()}\n`);
  process.exit(2);
}

function help(): string {
  return `tokn terminal pet

Usage:
  bun run start
  bun run start -- --stdin
  bun run start -- --demo
  bun run start -- --snapshot <state>

Options:
  --stdin                 Read RelayEvent JSONL from stdin
  --demo                  Cycle through every pet state
  --snapshot <state>      Print one ANSI frame and exit
  --relay-bin <path>      Spawn an installed tokn-session-relay binary
  --protocol <mode>       auto, ansi, kitty, or kitty_file
  --name <name>           Change the displayed pet name
  --no-color              Disable truecolor output
  -h, --help              Show this help

Without --stdin, the pet builds Relay if needed and spawns its binary so stdin
remains available for q, Escape, and c.`;
}

function trimBlankLines(lines: string[]): string[] {
  const copy = [...lines];
  while (copy[0]?.trim().length === 0) {
    copy.shift();
  }
  while (copy.at(-1)?.trim().length === 0) {
    copy.pop();
  }
  return copy;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
