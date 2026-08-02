# Pi input bridge

`input-bridge.ts` is an opt-in Pi extension that exposes a local, session-scoped
Unix socket. A client such as terminal-pet can send a prompt to the existing
interactive Pi process, and Pi inserts it through `sendUserMessage()` so the
prompt appears in the current TUI session.

Load it explicitly while experimenting:

```sh
pi --extension /absolute/path/to/tokn-session/extensions/pi/input-bridge.ts
```

For regular use, place the extension in `~/.pi/agent/extensions/` so Pi
auto-discovers it. The bridge is only enabled for interactive (`tui`) sessions
with a persisted session file. JSON, print, RPC, and ephemeral sessions remain
unchanged.

## Discovery

The extension writes a short-lived descriptor beside the session file:

```text
<session-file>.tokn-input.json
```

The descriptor points to a Unix socket in the system temporary directory and
contains the session identity, process id, and a random capability token. The
descriptor and socket are removed during normal session shutdown. A stale
descriptor is harmless: clients must validate the session identity and treat a
failed connection as an unavailable bridge.

## Protocol

Requests and responses are one JSON object per line. Every request includes the
`protocol`, `token`, `session_id`, and `session_file` from the descriptor.

Status request:

```json
{"protocol":1,"type":"status","request_id":"r1","token":"...","session_id":"...","session_file":"..."}
```

Prompt request:

```json
{"protocol":1,"type":"prompt","request_id":"r2","token":"...","session_id":"...","session_file":"...","message":"Continue the task"}
```

The bridge returns `ready`, `accepted`, or `error` responses. `accepted` means
Pi accepted the prompt and started a turn; Relay remains responsible for
streaming the resulting events. The first version rejects prompts while Pi is
already streaming instead of silently queueing them.
