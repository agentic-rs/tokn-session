# Codex desktop input experiment

This directory contains an experimental client for the private local IPC router
used by Codex App. It is intentionally separate from the supported Codex
app-server API and is not yet connected to Terminal Pet.

The experiment never discovers or defaults to the user's real Codex App IPC
endpoint. Callers must pass an explicit Unix socket or Windows named-pipe
endpoint. The test harness creates an isolated endpoint and emulates the
desktop router's client discovery and thread-owner forwarding flow.

Codex currently uses these platform endpoints:

- macOS and Unix: `$CODEX_HOME/ipc/ipc.sock`
- Windows: `\\.\pipe\codex-ipc`

```sh
cd extensions/codex
bun run check
```

The currently observed desktop request is `thread-follower-start-turn` version
1. Its payload maps the rollout thread id to `conversationId` and wraps the
app-server-style text input in `turnStartParams`.

GitHub Actions runs the framing and transport harness on Linux, macOS, and
Windows. The Windows job verifies named-pipe initialization, owner discovery,
successful forwarding, and the successful response path. Fake-router error
responses are Unix-only because Bun 1.3.13 does not flush those server-side
named-pipe responses on Windows. These tests do not claim compatibility with a
real Codex App build.

This is an undocumented compatibility surface. A production transport must
fail closed when the endpoint, request version, or owning window is unavailable.
