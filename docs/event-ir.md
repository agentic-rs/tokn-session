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

`usage.kind` distinguishes `model_call`, `operation_total`, and
`session_snapshot`. Session snapshots replace previous values; never sum them
or add them to per-call/operation accounting. Operations can include multiple
model calls. IDs identify the source record/message and turn/step when known;
absent IDs are not inferred. `input_tokens` includes cached input; cache fields are
subsets. `reasoning_tokens` must not be added to `output_tokens`. Missing
optional counters mean unreported, not necessarily zero. `native` retains the
original usage object and any provider extensions, including costs.
`total_tokens` preserves reported totals, which can include provider estimates
not represented in the input/output counters.

DSH reports uncached input separately; the adapter sums uncached input, cache
reads, and cache writes with overflow checking. It prefers assembled usage,
otherwise the last streamed snapshot for that turn/step, emitting it at that
record's position. An assembled message without usage does not suppress the
streamed usage. This avoids counting the same model call twice.

Pi assistant usage is per-call. Tool-result, compaction, and branch-summary
usage are operation totals, keyed by the session entry ID. Like DSH, Pi adds
uncached input, cache reads, and cache writes with overflow checking.
`cacheWrite1h` is already included in cache writes and is not added again.

Codex persisted `token_count.info.total_token_usage` is a session snapshot,
not proof of a new model call. Cached input is already included in input.
Consecutive identical `info` objects are suppressed; changed snapshots,
including decreases and total-only context estimates, are retained unchanged.
`native` retains the full `info` object, including `last_token_usage` and the
context window. Changed rate limits are diagnostic metadata. Unavailable info
is metadata, not zero usage, and resets deduplication; rollback/compaction also
reset it. Malformed counters remain unknown. A live tail started at EOF emits
its first observed snapshot without reconstructing earlier calls.

OpenCode assistant turns emit at most one `model_call` usage event. When valid
`step-finish` parts are present, the last one is authoritative; the assistant
message's tokens are a fallback for incomplete or older rows. Cache reads and
writes are included in `input_tokens` once, while a provider-reported total is
kept unchanged. Invalid accounting remains a visible unknown record without
hiding the turn's message, tool, or reasoning content. Live `step_finish`
envelopes use the same rule when their token data is valid.

## Compaction

`compaction` is a context operation, never a user prompt, final assistant reply,
or turn completion. Its `state` is `requested`, `started`, `summary_generated`,
`completed`, `failed`, `interrupted`, or `skipped`. Only explicit provider
evidence advances the state; a generated summary alone need not be installed.
`provider_phase` retains timing categories such as `pre_request`, independently
of lifecycle state. Missing trigger, reason, or measurements are not inferred.
`summary_opaque` marks unreadable provider summary material.

| Provider | Persisted evidence |
| --- | --- |
| Codex | `compacted` checkpoints, legacy `context_compacted` notices, canonical completed `ContextCompaction` items, and opaque response compaction items. No persisted start signal. |
| Pi | Completed `compaction` entries: summary, first kept entry, and tokens before. Runtime auto-compaction start/end are not in the session JSONL. |
| OpenCode | V1 compaction parts request the operation; a linked assistant `summary: true` message supplies summary and completion/error. It is not a conversation reply. |
| ZCode | `compaction`/`timeline.context_compaction` parts carry explicit status and operation ID; `compact_summary` boundary messages carry summary and retention references. |
| DSH | `compaction/start`, `/summary`, `/end` plus the plugin's replacement `user/message`, correlated by `compactionId`. Only a successful end completes the operation. |

WorkBuddy is intentionally deferred. Tool-output pruning is separate from
summarizing compaction and is not relabeled as a completed compaction.

Measurements distinguish `context_before`, `context_after`, and
`replaced_context`. `estimated: null` means unspecified. DSH's shadowed-token
estimate is only the replaced span. ZCode's `truePostCompactTokenCount` estimates
rebuilt context; its `postCompactTokenCount` is summarizer usage and must not be
used as context-after size. Summary-call usage remains a separate `usage` event:
DSH identifies a model call only with `llmStreamCall: true` and `rawOutput`.

`compaction_operations` projects observations with the same provider/session/
operation ID into one card, keeping the first source index stable and retaining
all contributor indices for inspection. Anonymous observations remain separate.
Terminal state survives later summary enrichment. Codex's adjacent checkpoint
and completion notice may be paired across token accounting, using a checkpoint-
scoped adapter key when no window ID exists; other records break that correlation.
This projection never deletes earlier transcript rows
or reconstructs the compacted model context. Relay native data stays an optional
sibling; no `native` field is added to this event.

Provider evidence: pinned `vendor/codex` rollout policy and compaction code,
`vendor/pi` session-manager types, `vendor/opencode` V1 compaction handling,
and `vendor/dsh/packages/compaction/compaction/src/types.ts`/checkpoint code.
ZCode fixtures are based on the installed 3.7.3 bundle, not a captured real
compaction session; new shapes must retain the unknown fallback.

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

Reasoning may also explicitly state `redacted: true`. This means the provider
withheld the readable text; it is distinct from a hidden event. Redacted
reasoning remains visible as a safe event marker, while its content is not
retained for human rendering.

Pi `custom` records are opaque extension state, not messages. `custom_message`
records are system-role context messages with extension attribution, original
native content/details, and the provider's `display` flag in provenance.
Hidden content remains in JSONL; pretty output and browser rows show only a
placeholder, including for unsupported hidden custom-message shapes. Discord
does not publish extension messages. Hidden messages do not count toward the
relay's recent-message replay window. Recognized branch summaries,
labels, session names, leaf changes, and active tools are metadata.

Codex turn context, world state, inter-agent communication metadata,
and rollbacks are metadata. Compaction summaries are
not final assistant replies. Historical subagent ownership filtering runs
before accounting and metadata normalization. No provenance is inferred from
a session's source for otherwise unattributed messages.

## Adoption

DSH produces lifecycle, usage, metadata, and provenance. Codex preserves
task/turn start, completion, and abort as turn-scoped lifecycle with native
turn IDs; missing IDs remain unknown rather than fabricated. Abort retains its
existing error event as well. Pi, Codex, and
OpenCode all normalize usage; Pi and Codex also adopt metadata, and Pi supplies
extension provenance.
Terminal Pet ignores compaction, usage, metadata, and hidden content for activity/focus and
lease handling. Authoritative Pet runtime state remains follow-up work;
Pi historical logs do not contain the live agent/turn boundaries, so
those require a separate bridge feature. DSH relay/input remain separate work.
