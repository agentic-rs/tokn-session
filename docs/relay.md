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

This is a best-effort live feed, not a durable replication protocol: no ack,
resume cursor, or complete history on startup. JSONL offsets are not stable
across file truncation/replacement; whole-session deletion and database/file
replacement do not emit a complete set of tombstones. A future viewer store
must establish a snapshot/resync boundary before treating it as authoritative.
JSONL updates only decode appended complete lines. OpenCode still reloads
changed sessions (and can perform its existing correctness fallback); this
change does not add incremental SQLite parsing or remove viewer reloads.

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
