# Codex desktop input experiment

This directory contains an experimental client for the private local IPC router
used by Codex App. It is intentionally separate from the supported Codex
app-server API and is not yet connected to Terminal Pet.

The experiment never discovers or defaults to the user's real Codex App socket.
Callers must pass an explicit Unix socket path. The test harness creates its own
socket in a temporary directory and emulates the desktop router's client
discovery and thread-owner forwarding flow.

```sh
cd extensions/codex
bun run check
```

The currently observed desktop request is `thread-follower-start-turn` version
1. Its payload maps the rollout thread id to `conversationId` and wraps the
app-server-style text input in `turnStartParams`.

This is an undocumented compatibility surface. A production transport must
fail closed when the socket, request version, or owning window is unavailable.
