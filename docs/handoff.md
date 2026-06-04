# Handoff

`tokn-session` is a prototype Rust workspace for session interoperability across agent tools. It can currently list and show existing sessions from Pi, Codex, and OpenCode.

## Current CLI

Commands are top-level:

```sh
tokn-session list --source codex --limit 5
tokn-session show --source opencode <session-id> --format pretty
tokn-session show --source pi <session-id> --format jsonl
```

The old `tokn-session sessions list/show` shape is intentionally unsupported.

## Crate Layout

- `tokn-session-core`: shared session and event IR.
- `tokn-session-client`: source dispatch.
- `tokn-session-cli`: argument parsing and display.
- `tokn-session-pi`: Pi session reader and normalizer.
- `tokn-session-codex`: Codex session reader and normalizer.
- `tokn-session-opencode`: OpenCode session reader and normalizer.

## Provider Sources

- Pi reads JSONL from `~/.pi/agent/sessions` unless `--session-dir` is passed.
- Codex reads JSONL from `~/.codex/sessions` and `~/.codex/archived_sessions` unless `--session-dir` is passed.
- OpenCode reads SQLite from `~/.local/share/opencode/opencode.db` unless `--session-dir` is passed. It opens the database as immutable read-only so viewing sessions does not require writing SQLite sidecar files.

## Event IR

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

Reasoning is intentionally flat:

- `text`
- `summary`
- `encrypted_content`
- `signature`

Pretty rendering shows visible reasoning text and summaries, but does not display encrypted reasoning payloads. JSONL preserves encrypted reasoning in the IR.

## Important Decisions

- The project is named `tokn-session`, not `tokn-agent`.
- Commands are top-level because the whole tool is about sessions.
- Provider-native events are normalized for display first; exact round-trip preservation is not a current goal.
- Unknown events should remain visible enough to discover new provider shapes.
- OpenCode shell tools with nonzero `metadata.exit` are marked as errors even when OpenCode records the tool state as completed.

## Known Gaps

- No `create` command yet.
- No `attach` command yet.
- The CLI help path currently exits through the same error-printing path as other parser errors.
- Timestamps are provider-native strings/numbers today; there is no unified timestamp type yet.
- OpenCode support currently uses the legacy-compatible `message` and `part` tables seen in local data, not the newer `session_message` projection.

## How To Verify

Run:

```sh
cargo fmt
cargo check
cargo test
```

Useful smokes:

```sh
cargo run -p tokn-session-cli -- list --source codex --limit 1
cargo run -p tokn-session-cli -- list --source opencode --limit 1
cargo run -p tokn-session-cli -- show --source opencode <session-id> --format pretty
```

## Next Likely Work

- Add the first `create` path for invoking a provider and streaming normalized events.
- Decide how to represent live event streams versus loaded historical sessions.
- Add provider-specific tests or fixtures so real local session stores are not the only verification path.
- Consider splitting stable event IR notes into `docs/event-ir.md` once the IR changes again.
