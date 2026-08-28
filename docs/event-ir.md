# Event classification

`AgentEvent` is a display-oriented, provider-neutral stream. Native detail is
retained where useful; it is not an exact reconstruction of provider context.

## Lifecycle

`lifecycle` identifies a turn and optionally a step, with `scope`, `phase`, and
an optional `outcome`. DSH emits started/finished boundaries. A step's finished
phase means closed, not successful: its outcome remains absent. Turn outcomes
are `completed`, `cancelled`, `interrupted`, `blocked`, `failed`, or
`token_limit`. Native reasons remain available, including cancellation causes.
Failed turns also emit the existing `error` event for existing consumers.
New or malformed reasons remain unknown rather than being called successful.

## Usage

`usage` is one model-call snapshot, not a delta or cumulative session total.
It carries turn/step IDs and a message ID when the assembled message provided
the usage. `input_tokens` includes cached input; cache read/write fields are
subsets. `reasoning_tokens` must not be added to `output_tokens`. Missing
optional counters mean unreported, not necessarily zero. `native` retains the
original usage object and any provider extensions.

DSH reports uncached input separately; the adapter sums uncached input, cache
reads, and cache writes with overflow checking. It prefers assembled usage,
otherwise the last streamed snapshot for that turn/step, emitting it at that
record's position. An assembled message without usage does not suppress the
streamed usage. This avoids counting the same model call twice.

## Metadata and provenance

`metadata` means a recognized non-conversation record whose required envelope
and payload fields were validated. It carries a category, native type, compact
summary, and full native record. Known DSH title/configuration snapshots,
inbox edits, context/todo snapshots, and auxiliary model requests use this
category. Queue inserts are not delivered user messages. Incomplete known
stream structure is metadata, not a completed tool call.

Pretty output uses compact summaries. Browser expansion and JSONL retain
native diagnostic bodies. No general rule hides `unknown`: unfamiliar tags,
malformed shapes, and unsupported content retain the existing visible fallback,
regardless of the provider's `ignorable` flag.

Messages and reasoning may carry optional `provenance`: the native `source`,
`surface_op`, and `source_event_seqs`. It preserves plugin attribution and
context edits without duplicating the conversation as an unknown event.
Provenance does not apply surface edits to the chronological history.

## Adoption

DSH is the first producer of these three event families. Pi, Codex, and
OpenCode keep their existing normalization behavior for now; in particular,
Codex's dropped lifecycle/token-count records and OpenCode's historical
step records are not migrated by this change. Pet parsers tolerate new event
types, but authoritative runtime state handling and DSH relay/input remain
separate work.
