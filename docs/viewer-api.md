# Remote session viewer

The same frontend runs in Tauri or a browser. Desktop calls `viewer-core`
directly through Tauri commands. In browser mode, the Rust `viewer-api` hosts
the compiled frontend and exposes the HTTP/SSE adapter over the same origin.

```text
Desktop UI → Tauri commands ─┐
                            ├→ viewer-core → provider history / snapshot readers
Browser → viewer-api (UI + HTTP/SSE) ──────┘ ↖ Relay live feed (managed stdio)
```

`viewer-core` owns catalogs, history, paging, trajectories, native Inspector
detail, reconciliation, and the index scheduler. Relay owns provider live-feed
normalization and its stdout/ZeroMQ/managed-stdio transports. In automatic mode,
the viewer host starts its own headless Relay child, reads versioned JSONL from
stdout, and uses records to wake authoritative snapshot readers in core.
Polling recovers startup gaps and missed records; the live feed is not a history
store. Stdin EOF stops the child, including after parent death. No private TCP
listener is needed for this managed connection. This is an architectural
simplification, not a measured throughput claim.

## Run locally

From the repository root, build the reused frontend and start the viewer server
on the machine containing sessions:

```sh
pnpm --dir apps/viewer install --frozen-lockfile
pnpm --dir apps/viewer build
cargo run -p tokn-viewer-api
```

Open `http://127.0.0.1:5558`; the browser defaults to the server that delivered
the page. The connection screen remains available for selecting another
machine.
The API defaults to loopback with no token. Set `TOKN_VIEWER_TOKEN` to require
bearer authentication on every data and event endpoint. The browser keeps the
token in memory only; reloads require reconnecting.

During frontend development, `pnpm --dir apps/viewer dev` still runs Vite at
`http://localhost:1437`. Start `viewer-api` with `--allow-origin
http://localhost:1437`, then connect to `http://127.0.0.1:5558`.

For a remote machine, a loopback API plus an SSH tunnel is sufficient:

```sh
ssh -N -L 5558:127.0.0.1:5558 your-machine
```

Open `http://127.0.0.1:5558` after creating the tunnel. Same-origin requests do
not need CORS configuration. `--allow-origin` is only needed when Vite or a UI
loaded from another viewer host connects across origins; it must match that
frontend's exact origin. Repeat the flag for additional origins. There is no
wildcard option. Non-loopback `--bind` requires a token; use HTTPS termination
or a private encrypted tunnel when transmitting sessions across a network. The
server speaks HTTP and does not configure TLS or a hosting provider.

Options:

- `--bind 127.0.0.1:5558`: listening address; port `0` chooses a free port.
- `--web-root apps/viewer/dist`: compiled Vite directory containing
  `index.html`; `TOKN_VIEWER_WEB_ROOT` provides the same setting.
- `--allow-origin http://localhost:1437`: allowed frontend origin.
- `--index-path <file>`: defaults to `~/.tokn/sessions/index.sqlite`.
- `--native`: include provider-native Inspector records in automatic mode.
- `--local`: read/index history directly without a managed Relay child.
- `TOKN_VIEWER_TOKEN`: access token (also accepted through `--token`).

Provider roots use the existing environment overrides in [relay.md](relay.md).
Choose **Change machine** to close requests/subscriptions and clear the viewer
before connecting elsewhere. Only one machine is selected at a time. The
browser does not change the server's Relay configuration.

## API contract

`GET /api/v1/health` returns `{"version":1}`.
Commands are `POST /api/v1/<command>` with the same snake_case payload as the
Tauri adapter, normally `{"request":{...}}`. Commands without a request take
`{}`. Responses use the shared viewer-core models. Supported commands:

- `list_sessions`, `list_session_children`
- `load_event_page`, `load_trajectory_event_page`, `load_event_detail`
- `acknowledge_session_attention`
- `get_session_index_progress`, `retry_session_index`, `get_relay_status`

Session keys are admitted against the server's discovered catalog before any
history access. A syntactically valid key containing an arbitrary source path
does not grant access. The API allows at most 16 concurrent blocking requests,
32 SSE clients, and 1 MiB request bodies. Error responses contain `error`;
invalid JSON/body-limit responses may be plain text.

`GET /api/v1/events` is SSE with a `ready` handshake and 15-second heartbeats.
Named events match Tauri: `relay-changed`, `relay-status`,
`session-index-changed`, and `session-index-progress`. Events invalidate data;
they are not complete transcript payloads. A lagging subscriber disconnects.
The browser retries, then reloads the catalog and selected timeline so missed
notifications cannot leave stale data indefinitely. The browser shows its
connection state and preserves last-received data during temporary outages.
Ctrl-C closes event streams and stops the managed child.

The legacy local snapshot/follow protocol now lives in viewer-core and can be
started with `tokn-viewer-api snapshot --bind tcp://127.0.0.1:5557 [--native]`.
It remains loopback-only and is used by desktop External mode. The old
`tokn-session-relay serve` command reports migration guidance. This endpoint is
separate from the browser HTTP API and retains its version-1 framed protocol.
