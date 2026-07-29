# Terminal Pet

`@tokn/terminal-pet` is a Bun prototype that turns `tokn-session-relay`
events into a small animated terminal companion.

It follows the Codex pet state vocabulary:

- `running`
- `needs_input`
- `ready`
- `blocked`
- `idle` when no current state lease is active

The pet keeps one state per Relay topic. Its roster shows every active session
that fits, followed by sessions inferred Ready during the last five minutes.
The highest-priority session drives the large pet in automatic mode. Priority
is `needs_input`, `blocked`, `ready`, then `running`; overflow is reported
explicitly instead of silently disappearing. A manually selected session
remains visible when the roster overflows and drives the pet until automatic
focus is restored.

## Run

From this directory:

```sh
bun install
bun run start
```

By default the app builds the workspace Relay once if needed, then spawns the
resulting binary. This keeps stdin available for keyboard controls:

- `q` or Escape quits and stops the child Relay.
- Up/Down or `j`/`k` selects a session, including recently completed work.
- `a` returns to automatic highest-priority focus.
- `c` clears the selected session's notification.

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

`dev` cycles through every state with multiple active and recent demo sessions,
then reloads when TypeScript changes.

## Rendering

The app automatically uses Kitty graphics in Kitty, Ghostty, and WezTerm, and
the Kitty local-file protocol in iTerm2 3.6 or newer. It falls back to truecolor
ANSI half-block pixels elsewhere. Image output is disabled inside tmux and
Zellij; the explicit Kitty overrides do not bypass that safety fallback.

Wide terminals use one graphical pet beside the session roster. Narrow
terminals suppress the artwork and spend the available rows on the roster.
Each row includes a state glyph, session identity, latest activity, and age, so
the display remains useful without color.

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

Recently Ready work stays in the roster for five minutes, or until `c`
acknowledges the focused notification. This is observed-run history, not an
authoritative provider completion log: an assistant message finishing or a
goal reporting completion is used as a fallback signal. Relay starts existing
files at their snapshotted EOF, so the pet does not reconstruct earlier
completion history at startup.

## Artwork

The checked-in Hachiware artwork is prototype-only fan art generated for local
iteration. Hachiware and Chiikawa belong to their respective rights holders.
Replace these frames with an original character before publishing or
distributing this project.
