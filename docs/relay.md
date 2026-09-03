# Relay records

`tokn-session-relay stdout --format json` emits one `RelayRecord` per JSONL
line. ZeroMQ publishes the same JSON as its second frame; the first frame is
the unchanged `provider.session_id` topic. Use `--native` with either output
mode to include provider data alongside the normalized events.

```json
{
  "path": "/sessions/pi.jsonl",
  "topic": "pi.session-1",
  "session": { "provider": "pi", "session_id": "session-1" },
  "operation": "upsert",
  "record_id": "jsonl:128",
  "native": { "type": "message", "id": "message-1", "message": {} },
  "events": []
}
```

The example abbreviates session context and native content. `events` contains
zero or more ordered `AgentEvent` objects. One native record may produce
reasoning, messages, tools, and usage together. An empty batch is valid, even
without native data. Batches follow source records, not a timer or arbitrary
number of messages. Replay windows retain entire records, including events
before the selected message within the same record.

`native` is omitted by default (`RelayConfig.include_native = false`), and is
not added to every `AgentEvent` variant. Existing native/provenance fields in
the IR remain unchanged. Opting in can expose additional provider metadata;
neither mode is a redaction or security boundary.

## Source boundaries and identity

Record keys are scoped by `(path, topic, record_id)`. An `upsert` replaces the
whole batch at that key, rather than appending every event again.

- Codex/Pi: one complete JSONL line, decoded once. Native data is the parsed
  original JSON value, including unknown fields (not original whitespace).
  `jsonl:<byte-offset>` identifies the line in the current file. Partial lines
  remain buffered until newline; malformed lines produce warnings.
- OpenCode: a consistent read-only SQLite session snapshot is normalized into
  a session record and one record per message, with ordered hydrated parts.
  IDs are `session:<id>` and `message:<id>`. Native data is the decoded row
  structure (`data`, `parts`, IDs and timestamps), not every SQL column or a
  SQLite change feed. Any change to the batch, or to requested native data,
  republishes the record. Removed message records in an observed session emit
  `operation: "remove"`, an empty batch, and no native payload.

The reader shares its grouped normalization with historical OpenCode/ZCode
loading; historical APIs still flatten to the existing `LoadedSession` shape.
Relay provider support remains Codex, Pi and OpenCode.

The stdout/ZeroMQ output is a best-effort live feed, not a durable replication protocol: no ack,
resume cursor, or complete history on startup. JSONL offsets are not stable
across file truncation/replacement; whole-session deletion and database/file
replacement do not emit a complete set of tombstones. Viewers should use the
separate snapshot/follow service below instead of treating PUB as authoritative.
JSONL updates only decode appended complete lines. OpenCode still reloads
changed sessions (and can perform its existing correctness fallback); this
wire migration does not add incremental SQLite parsing.

## Local snapshot/follow service

Start a separately configured service for the viewer:

```sh
tokn-session-relay serve --bind tcp://127.0.0.1:5557 --native
```

`--native` is optional. The usual `--codex-dir`, `--pi-dir`, and
`--opencode-dir` source overrides apply. `--poll-interval` defaults to 500ms
for active sessions; the shared metadata catalog is cached for two seconds.
`serve` always supplies complete history and rejects replay-window flags.
It does not start a PUB socket: pets can continue using an independently
configured `zeromq` process on port 5556, without changing their wire format.

This endpoint is **length-prefixed JSON over TCP, not ZeroMQ**. Each frame is
a big-endian u32 byte length followed by UTF-8 JSON. Version 1 requests are
`{"version":1,"action":"catalog"}` or
`{"version":1,"action":"follow","session_key":"<catalog key>"}`.
The server answers `hello` with its version, providers and native capability.
Catalog responses contain `header` frames followed by `catalog_end`, including
discovery warnings. Follow requests accept only keys from the discovered
catalog, never arbitrary paths.

A follow connection receives `begin`, zero or more `record` frames, then
`commit`. Begin and commit carry matching generation and decimal-string
revision values; begin also carries the session header and `reset` flag.
The first transaction has `reset: true`. Subsequent JSONL transactions append
new records in the same generation. File replacement, truncation and detected
same-length rewrites start a fresh generation and replace the entire snapshot.
OpenCode DB/WAL changes reconcile raw rows in one read transaction. Unchanged
message records reuse decoded data and normalization checkpoints; changes to
model state recompute dependent records until the state converges. SQLite rows
are still scanned because timestamps/counts cannot reliably identify edits.
Unrelated writes and WAL checkpoints publish nothing when the session is
unchanged. A true appended suffix stays in the same generation; edits,
deletions, reordering or database replacement reset the snapshot. Header-only
changes can commit without record frames. With `--native`, changes to existing
native data (including session update timestamps) also require a reset under
the v1 append/reset protocol. This does not change stdout/ZeroMQ loading.

Session bodies are loaded on demand. Concurrent subscribers share one reader
and normalizer per session; complete appended JSONL lines decode only once.
An idle session is released after its last subscriber leaves. Source errors,
invalid frames and interrupted transactions never commit partial data.
Slow clients are disconnected and reconnect to a complete snapshot; there is
no durable event journal or resume-after-restart cursor. Heartbeats are sent
every two seconds. Coalesced notifications include every intervening append.

Limits: numeric loopback addresses only (no remote authentication), 64 client
connections, 16 active sessions, 8 MiB per wire frame, 128 MiB serialized
records per snapshot, 128 MiB per JSONL source and 128 MiB of raw OpenCode
session-row payloads per cached reader. These are payload limits,
not exact process-RAM caps; decoded objects and snapshots add overhead. Large
sessions fail explicitly rather than silently returning partial history.
Any local process can connect; `--native` may expose sensitive provider data.

### Viewer connection

The viewer defaults to **Automatic Relay**: it launches a headless child of its
own executable, running this same service on an OS-assigned loopback port. The
shipped app needs no separate Relay installation or PATH lookup. The child uses
the same provider-root environment overrides as local history. A readiness pipe
reports the bound endpoint; the private port never replaces the saved external
endpoint. Codex roots verified as the active home's `sessions` or
`archived_sessions` retain that home's title/preview metadata without parsing
transcript bodies. Unrelated explicit directories do not inherit this metadata.
The app closes a lifetime pipe and reaps its child on exit/mode changes;
the child also exits on EOF if its parent crashes. Startup has a ten-second
timeout. Failures/crashes get at most three launch attempts with one-/two-second
backoff, then show Failed with an explicit Retry. Other Relay processes are
never stopped.

The panel persists `mode` (`automatic`, `external`, or `local`), external
`endpoint`, and `include_native` in app config `relay.json`. Native is off by
default and optional for Automatic; External uses the service's capability.
Legacy `enabled: true` settings migrate to External with the same endpoint;
explicitly disabled settings migrate to Local. Missing settings use Automatic.
Invalid settings produce a visible error instead of silent fallback.

External connects to a separately started `tokn-session-relay serve` and never
owns that process. Local explicitly clears Relay snapshots, stops any owned
child, and wakes the local catalog/index path. Switching mode, external endpoint,
or native inclusion clears snapshots to avoid mixing data sources.
Catalog polling updates the sidebar; up to eight recently opened sessions share
received snapshots between timeline, trajectories and Inspector. Covered providers
bypass local body indexing/reading. Uncovered providers retain local history.
Connection failures and automatic child restarts retain last-good committed
data without local fallback. Catalog and session connections are cancelled
before reconnecting to a restarted child's new port.

Live updates refresh the loaded event window and expanded trajectory items,
including while reading older events. Scrolling follows only at the bottom;
otherwise the reading position is preserved and “New activity” jumps to the
latest page. Each refresh uses one pinned snapshot for its bounded pages.
Append refreshes preserve expansion keys, while a replacement
generation clears them. Optional native records are available in bounded
Inspector details, excluding records containing redacted reasoning. Local
durable unread/seen indexing is not applied to Relay-backed sessions yet.

## Consumer migration

This intentionally replaces the old wire `event: AgentEvent` field with
`events: AgentEvent[]`; update external consumers together with Relay.
`TailUpdate.records` is the Rust library batch. JSON and ZeroMQ use the same
contract; human output still renders every normalized event separately.

All bundled pets use `apps/shared/relay.ts`. It validates a whole record before
dispatching its events sequentially, retaining path/session context and
awaiting each worker for backpressure. Empty batches do not wake workers.
Workers and rules still use internal single-event `RelayEvent` values; the
shared adapter is used by standalone pets and the supervisor. They ignore
native data and removals and remain activity observers, not record stores or
history-reconciliation clients. A bounded 4,096-record activity cache suppresses
unchanged OpenCode event slots when a snapshot is updated (including native-only
edits); cache eviction can allow replay and is not durable deduplication.
The default spawned Relay does not request
native data. Legacy wire envelopes are rejected rather than guessed.
