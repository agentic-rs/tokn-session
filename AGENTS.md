# Agent Notes

This repository is `tokn-session`, a Rust CLI and library workspace for viewing and eventually creating/attaching agent sessions across multiple providers.

## Current Shape

- The CLI binary is `tokn-session`.
- Commands are top-level: `list` and `show`. Do not reintroduce a `sessions` namespace.
- Supported sources today are `pi`, `codex`, and `opencode`.
- The current product focus is session interoperability: read provider-native session stores, normalize them into `AgentEvent`, and render/export them.
- Future likely commands are `create` and `attach`.

## Workspace

- `crates/core`: shared IR types such as `AgentEvent`, `SessionRef`, and `LoadedSession`.
- `crates/client`: source dispatch and public client API.
- `crates/cli`: argument parsing and terminal rendering.
- `crates/pi`: Pi JSONL session source and normalization.
- `crates/codex`: Codex JSONL session source and normalization.
- `crates/opencode`: OpenCode SQLite session source and normalization.
- `vendor/`: source-of-truth checkouts for upstream projects. Do not edit vendored code unless explicitly asked.

## Commands

```sh
cargo run -p tokn-session-cli -- list --source codex --limit 5
cargo run -p tokn-session-cli -- show --source opencode <session-id> --format pretty
cargo run -p tokn-session-cli -- show --source pi <session-id> --format jsonl
```

## Verification

After code changes, run:

```sh
cargo fmt
cargo check
cargo test
```

Also run at least one relevant CLI smoke. For example:

```sh
cargo run -p tokn-session-cli -- list --source opencode --limit 1
```

When changes are verified, commit them with a conventional commit message such as `feat: ...`, `fix: ...`, `refactor: ...`, or `docs: ...`.

## Style

- Prefer Rust.
- Use 2 spaces for indentation.
- Prefer code quality over minimizing diff size.
- Treat examples as representative; check similar provider code before changing only one source.
- Keep docs current but small. `docs/handoff.md` should describe the present state, not every historical step.
