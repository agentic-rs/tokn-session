#!/usr/bin/env bun

import { resolve } from "node:path";

import {
  IMAGE_PROTOCOLS,
  PetImageController,
  resolveImageProtocol,
  type ImageProtocolOption
} from "./image_protocol";
import { TerminalKeyDecoder, type PetKeyAction } from "./keys";
import { loadPetArt, selectPose } from "./art";
import { consumeJsonl, JsonlDecoder } from "./jsonl";
import { focusSnapshot, moveFocusTopic, type FocusDirection } from "./navigation";
import { parseRelayEvent, type RelayEvent } from "./protocol";
import { renderScreen, type RenderMeta } from "./renderer";
import {
  PiInputBroker,
  TerminalInputEditor,
  type TerminalInputEvent
} from "./input";
import {
  PET_STATES,
  PetStore,
  type PetFocus,
  type PetSnapshot,
  type PetState
} from "./state";
import { TerminalSurface } from "./terminal";

type RunMode = "relay" | "stdin" | "demo" | "snapshot";

const KEY_SEQUENCE_TIMEOUT_MS = 50;

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
        : "relay",
    control_mode: runOptions.mode === "stdin" || !process.stdin.isTTY
      ? "signal_only"
      : runOptions.mode === "demo"
        ? "demo"
        : "relay"
  };
  const startedAt = Date.now();
  const sourceAbort = new AbortController();
  const keyDecoder = new TerminalKeyDecoder();
  const inputEditor = new TerminalInputEditor();
  const piInput = new PiInputBroker();

  let child: RelayChild | undefined;
  let stopped = false;
  let exitCode = 0;
  let frameTimer: ReturnType<typeof setInterval> | undefined;
  let keyFlushTimer: ReturnType<typeof setTimeout> | undefined;
  let selectedTopic: string | undefined;
  let inputTopic: string | undefined;
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
    if (keyFlushTimer) {
      clearTimeout(keyFlushTimer);
      keyFlushTimer = undefined;
    }
    inputEditor.cancel();
    inputTopic = undefined;
    if (diagnostic) {
      meta.diagnostic = diagnostic;
      render();
    }
    sourceAbort.abort();
    resolveStopped?.();
  };

  const baseSnapshot = (nowMs: number): PetSnapshot => runOptions.mode === "demo"
    ? demoSnapshot(startedAt, nowMs)
    : store.snapshot(nowMs);

  const snapshotAt = (nowMs: number): PetSnapshot => {
    const snapshot = baseSnapshot(nowMs);
    if (
      selectedTopic
      && !snapshot.sessions.some((session) => session.topic === selectedTopic)
    ) {
      selectedTopic = undefined;
    }
    return focusSnapshot(snapshot, selectedTopic);
  };

  const render = (): void => {
    const nowMs = Date.now();
    const snapshot = snapshotAt(nowMs);
    const pose = selectPose(snapshot.state, snapshot.state_changed_at, nowMs);
    const screen = renderScreen(snapshot, art[pose].ansi, {
      ...meta,
      stats: decoder.stats,
      focus_mode: selectedTopic ? "manual" : "auto",
      input_active: inputEditor.active,
      input_line: inputEditor.active ? inputEditor.value : undefined
    }, {
      columns: process.stdout.columns ?? 80,
      rows: process.stdout.rows ?? 24,
      color: runOptions.color,
      image_protocol: imageProtocol,
      name: runOptions.name,
      now_ms: nowMs
    });
    surface.render(screen.lines);
    if (screen.image_anchor) {
      process.stdout.write(imageController.draw(pose, screen.image_anchor));
    } else {
      process.stdout.write(imageController.clear());
    }
  };

  const onResize = (): void => {
    if (stopped) {
      return;
    }
    process.stdout.write(imageController.clear());
    surface.invalidate();
    render();
  };
  const onSignal = (): void => stop(0);
  const moveFocus = (direction: FocusDirection): void => {
    if (inputEditor.active) {
      return;
    }
    const snapshot = baseSnapshot(Date.now());
    selectedTopic = moveFocusTopic(snapshot, selectedTopic, direction);
    render();
  };
  const beginInput = (): void => {
    if (runOptions.mode !== "relay") {
      meta.input_status = "input requires live Relay mode";
      render();
      return;
    }
    const focus = snapshotAt(Date.now()).focus;
    if (!focus) {
      meta.input_status = "no session is selected";
      render();
      return;
    }
    const provider = focus.provider ?? focus.topic.split(".", 1)[0];
    if (provider?.toLowerCase() !== "pi") {
      meta.input_status = "terminal input currently supports Pi sessions only";
      render();
      return;
    }
    inputTopic = focus.topic;
    meta.input_status = undefined;
    meta.diagnostic = undefined;
    inputEditor.begin();
    render();
  };
  const handleInputEvents = (events: TerminalInputEvent[]): void => {
    for (const event of events) {
      switch (event.type) {
        case "changed":
          render();
          break;
        case "cancelled":
          inputTopic = undefined;
          meta.input_status = undefined;
          render();
          break;
        case "submitted": {
          const topic = inputTopic;
          inputTopic = undefined;
          if (event.text.trim().length === 0) {
            meta.input_status = "message cannot be empty";
            render();
            break;
          }
          meta.input_status = "sending input to Pi…";
          render();
          if (!topic) {
            meta.input_status = "no session is selected";
            render();
            break;
          }
          void piInput.submit(topic, event.text).then(
            () => {
              if (!stopped) {
                meta.input_status = "Pi input sent";
                render();
              }
            },
            (error: unknown) => {
              if (!stopped) {
                meta.input_status = undefined;
                meta.diagnostic = errorMessage(error);
                render();
              }
            }
          );
          break;
        }
      }
    }
  };
  const dispatchActions = (actions: PetKeyAction[]): void => {
    for (const action of actions) {
      switch (action) {
        case "quit":
          stop(0);
          return;
        case "acknowledge":
          {
            const topic = snapshotAt(Date.now()).focus?.topic;
            if (topic) {
              store.acknowledge(topic);
              render();
            }
          }
          break;
        case "select_next":
          moveFocus("next");
          break;
        case "select_previous":
          moveFocus("previous");
          break;
        case "auto_focus":
          selectedTopic = undefined;
          render();
          break;
        case "begin_input":
          beginInput();
          break;
      }
    }
  };
  const onKey = (chunk: Buffer): void => {
    if (stopped) {
      return;
    }
    if (keyFlushTimer) {
      clearTimeout(keyFlushTimer);
      keyFlushTimer = undefined;
    }
    for (let index = 0; index < chunk.length && !stopped; index += 1) {
      if (inputEditor.active) {
        handleInputEvents(inputEditor.feed(chunk.subarray(index)));
        break;
      }
      dispatchActions(keyDecoder.push(chunk.subarray(index, index + 1)));
    }
    if (!stopped && !inputEditor.active && keyDecoder.has_pending_sequence) {
      keyFlushTimer = setTimeout(() => {
        keyFlushTimer = undefined;
        if (!stopped) {
          dispatchActions(keyDecoder.flush());
        }
      }, KEY_SEQUENCE_TIMEOUT_MS);
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
          if (!stopped) {
            piInput.observe(event);
            store.ingest(event);
            render();
          }
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
            if (!stopped) {
              piInput.observe(event);
              store.ingest(event);
              render();
            }
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
    if (keyFlushTimer) {
      clearTimeout(keyFlushTimer);
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
    if (meta.diagnostic) {
      process.stderr.write(`${sanitizeDiagnostic(meta.diagnostic)}\n`);
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
  if (!runOptions.relay_bin) {
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
    source_label: "snapshot",
    control_mode: "none"
  }, {
    columns: 64,
    rows: 22,
    color: runOptions.color,
    image_protocol: "ansi",
    name: runOptions.name,
    now_ms: nowMs
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
  const completedAt = state === "ready"
    ? stateChangedAt
    : undefined;
  const root: PetFocus = {
    topic: "codex.demo",
    root_topic: "codex.demo",
    depth: 0,
    is_provisional: false,
    state: "idle",
    family_state: state,
    family_last_event_at: stateChangedAt,
    state_changed_at: stateChangedAt,
    last_event_at: stateChangedAt,
    label: state === "idle" ? "No active agents" : "Coordinating subagents",
    provider: "codex",
    project_label: "tokn-agent",
    title: "Terminal pet roster",
    session_id: "demo",
    agent: "root",
    recently_completed: false,
    descendant_count: state === "idle" ? 1 : 2,
    active_descendant_count: state === "idle" ? 0 : 1,
    urgent_descendant_count: state === "needs_input" || state === "blocked" ? 1 : 0,
    recent_descendant_count: 1
  };
  const worker: PetFocus = {
    topic: "codex.worker",
    parent_topic: root.topic,
    root_topic: root.topic,
    depth: 1,
    is_provisional: false,
    state,
    family_state: state,
    family_last_event_at: stateChangedAt,
    state_changed_at: stateChangedAt,
    last_event_at: stateChangedAt,
    label: demoLabel(state),
    provider: "codex",
    project_label: "tokn-agent",
    title: "Improve session show",
    session_id: "worker",
    agent: "Avicenna",
    completed_at: completedAt,
    recently_completed: completedAt !== undefined,
    outcome: completedAt === undefined ? undefined : "completed",
    descendant_count: 0,
    active_descendant_count: 0,
    urgent_descendant_count: 0,
    recent_descendant_count: 0
  };
  const recent: PetFocus = {
    topic: "codex.recent",
    parent_topic: root.topic,
    root_topic: root.topic,
    depth: 1,
    is_provisional: false,
    state: "idle",
    family_state: "idle",
    family_last_event_at: stateChangedAt - 45_000,
    state_changed_at: stateChangedAt - 45_000,
    last_event_at: stateChangedAt - 45_000,
    label: "Updated the Relay documentation",
    provider: "codex",
    project_label: "tokn-agent",
    title: "Document Relay events",
    session_id: "recent",
    agent: "Meitner",
    completed_at: stateChangedAt - 45_000,
    recently_completed: true,
    outcome: "completed",
    descendant_count: 0,
    active_descendant_count: 0,
    urgent_descendant_count: 0,
    recent_descendant_count: 0
  };
  const sessions = state === "idle"
    ? [root, recent]
    : [root, worker, recent];
  const focus = state === "idle" ? recent : worker;
  return {
    state: focus.state,
    state_changed_at: focus.state_changed_at,
    active_sessions: state === "idle" ? 0 : 1,
    total_sessions: 3,
    sessions,
    focus
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

Without --stdin, the pet builds Relay if needed and spawns its binary. Use
Up/Down or j/k to select a session, a for automatic focus, c to clear its
notification, and q or Escape to quit.`;
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

function sanitizeDiagnostic(value: string): string {
  return Bun
    .stripANSI(value)
    .replaceAll(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g, "")
    .replaceAll(/\s+/g, " ")
    .trim();
}
