# Handoff

Read `AGENTS.md` first for the project goal, stable architecture, and working rules. This file tracks volatile implementation status and context a future AI would otherwise need to rediscover.

## Current Status

`tokn-session` can list and show existing sessions from Pi, Codex, and OpenCode.

Implemented CLI:

```sh
tokn-session list --source codex --limit 5
tokn-session show --source opencode <session-id> --format pretty
tokn-session show --source pi <session-id> --format jsonl
tokn-session browse --source codex <session-id>
tokn-session create --source opencode --executor "tokn-gateway proxy opencode --npx --" "create a todo app"
tokn-session append --source opencode --executor "tokn-gateway proxy opencode --npx --" --session <session-id> "next turn"
tokn-session append --source opencode --executor "tokn-gateway proxy opencode --npx --" --continue "next turn"
tokn-session-relay zeromq
tokn-session-relay stdout
```

The old `tokn-session sessions list/show` shape is intentionally unsupported.

## Session Relay

`tokn-session-relay` follows active Codex and Pi JSONL session trees. It requires
an output subcommand:

```sh
tokn-session-relay zeromq --bind tcp://127.0.0.1:5556
tokn-session-relay stdout
```

Both modes watch `~/.codex/sessions` and `~/.pi/agent/sessions` by default and
seed existing files from their session header before following from the
snapshotted EOF. Their historical bodies are not replayed. `--codex-dir`,
`--pi-dir`, `--poll-interval`, `--replay=<count>`, and `--replay-all` are shared
options.

Native filesystem watching is registered between the initial file snapshot and
the EOF-seeding pass, so appends during startup remain visible. The periodic
scan is a fallback for missed notifications and roots created after startup.
Watcher notifications retain and coalesce their affected paths, so normal
updates inspect only changed files instead of rescanning every session. macOS
uses the kqueue backend because FSEvents can omit these session-file writes.
Newly discovered or replaced files emit all normalized events beginning at the
third-most-recent message by default. `--replay=<count>` changes that window,
while `--replay-all` emits every complete record. These replay options only
apply to files discovered or replaced after startup.

`stdout` writes one normalized `AgentEvent` JSON object per line and flushes
after every event. Diagnostics remain on stderr.

`zeromq` binds `tcp://127.0.0.1:5556` by default. Each publication is a two-frame
ZeroMQ message:

1. `codex.<session_id>` or `pi.<session_id>` topic
2. normalized `AgentEvent` JSON

The relay publishes all normalized events, including reasoning, tool calls,
errors, lifecycle events, and unknown provider-native shapes. It buffers partial
JSONL records, discovers newly created files, handles truncation/replacement,
and combines native filesystem notifications with a periodic rescan.

The reusable relay loop lives in the library as `SessionRelay`. `RelayConfig`
controls provider roots, new-file replay, and the periodic recovery interval.
Library consumers call `next_update().await`; notification and scan failures
that can be retried are returned as warnings in `TailUpdate`.

## Provider Sources

- Pi reads JSONL from `~/.pi/agent/sessions` unless `--session-dir` is passed.
- Codex reads JSONL from `~/.codex/sessions` and `~/.codex/archived_sessions` unless `--session-dir` is passed.
- OpenCode reads SQLite from `~/.local/share/opencode/opencode.db` unless `--session-dir` is passed.
- OpenCode opens its database with an immutable read-only SQLite URI so viewing sessions does not require writing sidecar files.

## Event IR Status

The shared IR is `AgentEvent`.

Current event families include:

- `session_started`
- `provider_changed`
- `message`
- `reasoning`
- `goal_updated`
- `tool_call`
- `error`
- `unknown`

Tool calls now carry semantic display metadata:

- `tool_kind`: `shell`, `file_read`, `file_write`, `file_edit`, `search`, `web`, `task`, or `unknown`
- `summary`: compact facts for known tool families, such as shell command/exit code or file edit path and rough line counts

Raw `input` and `output` remain in the IR for debugging and provider-native detail.

Reasoning is intentionally flat:

- `text`
- `summary`
- `encrypted_content`
- `signature`

Pretty rendering shows visible reasoning text and summaries, but does not display encrypted reasoning payloads. JSONL preserves encrypted reasoning in the IR.

Reusable display formatting lives in `crates/render`. It depends on `core`, not on terminal libraries. The CLI uses it for linear output and the interactive browser uses its `EventDisplay` rows for collapsed summaries and expanded detail.

Pretty rendering also prefers compact semantic tool lines, such as:

```text
shell cargo test #call_abc
edit crates/core/src/agent_event.rs +4 -1 #call_abc
read crates/cli/src/render.rs #call_abc
```

Unknown tools still render their raw input/output so new provider shapes remain discoverable.
Unknown events preserve raw provider-native payloads when available and pretty rendering shows that native payload.

`browse` is the first interactive historical-session view. Without a session id, it opens an alternate-screen session list; Enter opens the selected session. With a session id, it opens the event browser directly. The event browser uses one row per normalized event. Rows are collapsed by default; expanded rows reuse the same per-event pretty rendering as linear output.

Current browser keys:

- `j`/Down and `k`/Up move the selected event row.
- `h` collapses the selected row; `l` expands it.
- Enter/Space toggles expansion.
- In the session list, Enter opens the selected session.
- `z` expands only the selected row.
- `C` collapses all rows.
- `g`/Home and `G`/End jump to the first/last event.
- Ctrl-D/Ctrl-U move by a coarse page.
- In the event browser opened from the session list, `q`/Esc returns to the session list.
- In direct event browsing and the session list, `q`/Esc quits.

## Current Decisions And Edges

- OpenCode shell tools with nonzero `metadata.exit` are marked as errors even when OpenCode records the tool state as completed.
- Tool kind classification and summary extraction live in `crates/core`; provider normalizers should use the shared helpers where possible.
- OpenCode support currently uses the legacy-compatible `message` and `part` tables seen in local data, not the newer `session_message` projection.
- Codex native JSONL parsing uses the published `codex-protocol` crate where lines match its `RolloutItem` definitions. Some local/vendor-current shapes are newer or looser than the published crate, so the Codex source keeps compatibility fallbacks for existing loose fixture shapes and unknown-event discovery.
- Codex `event_msg.thread_goal_updated` maps to the visible `goal_updated` IR event.
- Timestamps are provider-native strings/numbers today; there is no unified timestamp type yet.
- The CLI help path currently exits through the same error-printing path as other parser errors.

## Print Invocation Status

`create` and `append` have an initial configurable executor path. They do not assume provider binaries are installed. Pass `--executor <launcher>` or set `TOKN_SESSION_<SOURCE>_EXECUTOR`, such as `TOKN_SESSION_OPENCODE_EXECUTOR`.

The executor is only the launcher, equivalent to the provider binary. Provider-specific print-mode arguments are added by the source adapter. For OpenCode, `create` appends `run --format json <prompt>`, so gateway-style commands look like:

```sh
tokn-session create --source opencode --executor "tokn-gateway proxy opencode --npx --" "create a todo app"
```

`append` supports exactly one target:

```sh
tokn-session append --source opencode --executor "tokn-gateway proxy opencode --npx --" --session <session-id> "next turn"
tokn-session append --source opencode --executor "tokn-gateway proxy opencode --npx --" --continue "next turn"
```

Advanced custom executors may include an argv that is exactly `{prompt}`; in that case the executor is treated as the full command and no provider-specific args are appended.

`--cwd <dir>` runs the executor from a specific working directory.

Current limitation: provider output is inherited directly from the child process. The shared `LiveSessionEvent` envelope now exists in `crates/core`, and `crates/render` can pretty-render live events, but the CLI print path does not consume it yet.

OpenCode has the first live-output normalizer: `OpenCodeLiveNormalizer` parses `opencode run --format json` JSONL envelopes into `LiveSessionEvent`. It currently maps `text`, `reasoning`, `tool_use`, and `error` into normalized `AgentEvent`s and preserves other live envelopes such as `step_start` as unknown native events.

## Known Gaps

- No `attach` command yet.
- Codex has an initial normalization fixture test. Pi/OpenCode provider fixtures and CLI golden tests are still missing.
- The relay uses ZeroMQ `PUB/SUB`, which intentionally has no persistence or
  delivery acknowledgement; subscribers that are disconnected can miss events.

## Useful Smokes

GitHub Actions CI runs format check, workspace check/test, and CLI build on pushes to `main` and pull requests.

```sh
cargo run -p tokn-session-cli -- list --source codex --limit 1
cargo run -p tokn-session-cli -- list --source opencode --limit 1
cargo run -p tokn-session-cli -- show --source opencode <session-id> --format pretty
cargo run -p tokn-session-relay -- stdout
cargo run -p tokn-session-relay -- zeromq
```

## Next Likely Work

- Wire `create`/`append` stdout through provider live normalizers instead of inheriting child stdout directly.
- Decide whether live stream consumption should live in `client` as callbacks/iterators or in the CLI command path.
- Extend provider fixture coverage beyond Codex, especially Pi JSONL and OpenCode SQLite normalization.
- Add CLI golden tests for tiny fixture-backed `list` and `show` outputs.
- Consider splitting stable event IR notes into `docs/event-ir.md` once the IR changes again.
