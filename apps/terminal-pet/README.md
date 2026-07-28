# Terminal Pet

`@tokn/terminal-pet` is a Bun prototype that turns `tokn-session-relay`
events into a small animated terminal companion.

It follows the Codex pet state vocabulary:

- `running`
- `needs_input`
- `ready`
- `blocked`
- `idle` when no current state lease is active

The pet keeps one state per Relay topic, then displays the highest-priority
session. Priority is `needs_input`, `blocked`, `ready`, then `running`.

## Run

From this directory:

```sh
bun install
bun run start
```

By default the app builds the workspace Relay once if needed, then spawns the
resulting binary. This keeps stdin available for `q`, Escape, and `c`:

- `q` or Escape quits and stops the child Relay.
- `c` clears the focused notification.

To consume an existing JSONL pipeline:

```sh
cargo run -q -p tokn-session-relay -- stdout --format json \
  | bun run start -- --stdin
```

In pipeline mode, exit with Ctrl-C because stdin belongs to Relay.

For fast art and animation iteration:

```sh
bun run dev
bun run snapshot
```

`dev` cycles through every state and reloads when TypeScript changes.

## Rendering

The app automatically uses Kitty graphics in Kitty, Ghostty, and WezTerm, and
the Kitty local-file protocol in iTerm2 3.6 or newer. It falls back to truecolor
ANSI half-block pixels elsewhere. Image output is disabled inside tmux and
Zellij; the explicit Kitty overrides do not bypass that safety fallback.

Override detection with:

```sh
bun run start -- --protocol ansi
bun run start -- --protocol kitty
bun run start -- --protocol kitty_file
```

## Current state inference

Relay does not yet normalize Codex `task_started` and `task_complete` records,
so this prototype derives status from messages, tool calls, reasoning, errors,
goals, and preserved input-request events. The reducer is deliberately isolated
in `src/state.ts` so explicit lifecycle events can replace those heuristics.

## Artwork

The checked-in Hachiware artwork is prototype-only fan art generated for local
iteration. Hachiware and Chiikawa belong to their respective rights holders.
Replace these frames with an original character before publishing or
distributing this project.
