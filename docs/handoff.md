# Handoff

Read `AGENTS.md` first for the project goal, stable architecture, and working rules. This file tracks volatile implementation status and context a future AI would otherwise need to rediscover.

## Current Status

`tokn-session` can list and show existing sessions from Pi, Codex, and OpenCode.

Implemented CLI:

```sh
tokn-session list --source codex --limit 5
tokn-session show --source opencode <session-id> --format pretty
tokn-session show --source codex <session-id> --scope tree
tokn-session show --source pi <session-id> --format jsonl
tokn-session browse --source codex <session-id>
tokn-session create --source opencode --executor "tokn-gateway proxy opencode --npx --" "create a todo app"
tokn-session append --source opencode --executor "tokn-gateway proxy opencode --npx --" --session <session-id> "next turn"
tokn-session append --source opencode --executor "tokn-gateway proxy opencode --npx --" --continue "next turn"
tokn-session-relay zeromq
tokn-session-relay stdout
cd apps/discord-pet && bun run login
cd apps/discord-pet && bun run start
cd apps/pet && bun run start
cd apps/terminal-pet && bun run start
```

The old `tokn-session sessions list/show` shape is intentionally unsupported.

## Session Relay

`tokn-session-relay` follows active Codex and Pi JSONL session trees plus the
OpenCode SQLite database. It requires an output subcommand:

```sh
tokn-session-relay zeromq --bind tcp://127.0.0.1:5556
tokn-session-relay stdout --format summary
```

Both modes watch `~/.codex/sessions`, `~/.pi/agent/sessions`, and
`~/.local/share/opencode/opencode.db` by default. Existing JSONL files seed
their session header before following from the snapshotted EOF, while existing
OpenCode sessions are snapshotted without replay. `--codex-dir`, `--pi-dir`,
`--opencode-dir` (a data directory or database path), `--poll-interval`,
`--replay=<count>`, and `--replay-all` are shared options.

Native filesystem watching is registered between the initial file snapshot and
the EOF-seeding pass, so appends during startup remain visible. The periodic
scan is a 30-second fallback for missed notifications and roots created after
startup. Watcher notifications retain and coalesce their affected paths, so
normal updates inspect only changed files instead of rescanning every session.
OpenCode is watched non-recursively at its data directory plus the database and
SQLite WAL/SHM files; unrelated logs, snapshots, and auth files do not trigger
database work. macOS uses the kqueue backend because FSEvents can omit these
session-file writes.
Newly discovered or replaced files emit all normalized events beginning at the
third-most-recent message by default. `--replay=<count>` changes that window,
while `--replay-all` emits every complete record. These replay options only
apply to files discovered or replaced after startup.

`stdout` supports `--format pretty|summary|json` and defaults to `summary`.
Human-readable formats include the event timestamp, Codex Desktop project name
when available, abbreviated session id, and message id/parent when available.
Pretty output also prints the full session context before the first event
observed for each session. `--color` adds ANSI color to human output. JSON
remains colorless `RelayEvent` JSONL even when `--color` is present. Every
format flushes after each event, and diagnostics remain on stderr.

`zeromq` binds `tcp://127.0.0.1:5556` by default. Each publication is a two-frame
ZeroMQ message:

1. `codex.<session_id>`, `pi.<session_id>`, or `opencode.<session_id>` topic
2. serialized `RelayEvent` JSON

`RelayEvent` wraps the normalized `AgentEvent` with the source path, topic, and
`SessionContext`. Context includes session id, optional parent/title, cwd and
start time, optional agent path/nickname/role, plus a project object. That
object carries the distinct `project_name`, `folder_name`, and
`repository_name` fields as well as the existing folder path, repository URL,
branch, commit, and compatibility `name`. For Codex sessions, `project_name`
comes from Codex Desktop's optional `.codex-global-state.json`: direct thread
assignment wins, then parent-thread assignment, then the longest matching
workspace root. Relay reloads this catalog when the file changes. Missing or
malformed Desktop metadata does not stop Relay.
`folder_name` comes from the cwd basename and `repository_name` from the Git
remote. Agent metadata comes only from the first session header, including its
thread-spawn source when needed. Missing paths remain null for root and
subagent sessions; the relay does not derive `/root`. Title is never invented
when the provider file does not contain one.

Pretty session context shows `agent_path` only when it is present and not
`/root`. Summary lines include the same paths as `agent=<path>`. JSON preserves
the recorded value unchanged, including null or an explicit `/root`.

The relay publishes all normalized events, including reasoning, tool calls,
errors, lifecycle events, and unknown provider-native shapes. It buffers partial
JSONL records, discovers newly created files, handles truncation/replacement,
and combines native filesystem notifications with a periodic rescan. OpenCode
session summaries are cached, so a database notification reloads only new or
changed sessions on the normal path; when message/part timestamps cannot prove
which session changed, it performs one correctness fallback over the current
sessions. New sessions use the replay window, while changed or newly appended
message/part events are emitted once.
The database is opened read-only with WAL visibility and an immutable fallback;
the relay never runs provider migrations.

The reusable relay loop lives in the library as `SessionRelay`. `RelayConfig`
controls provider roots, new-file replay, and the periodic recovery interval.
Library consumers call `next_update().await`; notification and scan failures
that can be retried are returned as warnings in `TailUpdate`.

## Discord Pet

`apps/discord-pet` is a Bun/TypeScript application that consumes Relay JSONL
and mirrors root Codex and Pi conversations into Discord. By default it runs
an incremental workspace Relay build and spawns
`tokn-session-relay stdout --format json`; `--stdin` consumes an existing
pipeline instead. It reads `~/.tokn/pet/discord.yaml`, validates the bot and
configured guild/channel through Discord's REST API, and creates one public
thread per root session. It publishes root user messages and final assistant
messages only. Commentary, reasoning, tools, and child sessions are ignored.

The YAML contains `bot_token`, `guild_id`, and `channel_id`. Thread mappings are
persisted beside it, so later turns continue in the same thread after a process
restart. The default config uses `discord-state.json`; named configs derive
distinct state filenames. Discord embeds are split against the platform's
UTF-16 length accounting, mentions are disabled, transient requests and rate
limits are retried, and the token is never logged. The bot needs no privileged
intents.

`bun run login` from `apps/discord-pet` is the preferred configuration path. It
first walks through Guild Install and waits for the bot to appear in the server,
then prints where to obtain the bot token and Discord IDs. It hides token input,
asks before replacing an existing file, validates that the authenticated bot
can access the channel and that the channel belongs to the configured guild,
then writes the YAML with owner-only permissions. The validated identity is
also recorded as optional `bot_username`; configs created before that field
remain valid. `--config` overrides the destination.

Existing files start at their snapshotted EOF. Newly discovered files use the
relay's three-message replay window so the first prompt is not missed. See
`apps/discord-pet/README.md` for setup and permissions.

## Pet Supervisor

`apps/pet` is the high-level Bun supervisor. It owns one Relay stream, evaluates
declarative fan-out rules, and delivers matching `RelayEvent`s to bounded,
serial queues for in-process async worker objects. Downstream workers are not
subprocesses. The initial worker types are terminal and Discord; multiple named
Discord workers may each reference a different credential/channel YAML and use
independent persistent thread maps.

The checked-in `pet.example.yaml` sends the complete Relay stream to terminal
so its state inference retains tool, reasoning, error, and lifecycle context.
It sends root user messages and final assistant messages to `discord_volty`
only when `SessionContext.project.repository_name` matches the case-insensitive
glob `volty*`. Rules fan out, AND fields inside one `when`, OR values inside
arrays, deduplicate targets, and drop events with no match.

Workers expose `start`, `handle`, and `stop`. A failure handling one event is
reported without poisoning later queue work. The default per-worker capacity is
256; full queues backpressure Relay consumption rather than dropping events.
Terminal `q`/Escape aborts the shared source and shuts every worker down.

## Terminal Pet

`apps/terminal-pet` is a Bun/TypeScript prototype that consumes Relay JSONL and
shows one graphical terminal companion beside a multi-session roster. It runs
an incremental workspace Relay build, then spawns
`tokn-session-relay stdout --format json`, or accepts an existing stream with
`--stdin`.

The reducer keeps a session graph keyed by Relay topic. Root tasks are rendered
as project-labelled families, with active and recent subagents nested beneath
them by `parent_session_id`. Provisional child rows appear as soon as a parent
reports `agent_activity.started` and reconcile when the child's own Relay topic
arrives. Child urgency bubbles into the root summary while automatic focus
stays on the actionable child. `interacted` is only an annotation;
`interrupted` becomes a recent Interrupted outcome rather than falsely showing
Blocked. Stable agent activity is deduplicated by provider and event id, and
provider occurrence times prevent replayed old activity from looking current.

Within each family, state still uses
`needs_input > blocked > ready > running > idle`, followed by idle sessions
that were inferred Ready or Interrupted in the last five minutes. Up/Down or
`j`/`k` selects another session by topic, `a` restores automatic focus, and
`Enter` opens a composer for the focused session. A second `Enter` submits the
message and `Escape` cancels it. Input is currently Pi-only: the terminal
records the observed Relay path for each Pi topic and starts
`pi --mode json --session <path> --print` with the prompt on stdin. Pi writes
the session JSONL and Relay remains the only event/display source. Non-Pi or
unobserved sessions remain read-only. `c` acknowledges the selected
notification. Responsive text rows keep concurrent
and recent sessions visible; overflow windows around a manual selection so it
cannot disappear off-screen. Root labels prefer `project_name`, then folder
name, repository name, and the legacy inferred name. Child labels prefer agent
nickname, then agent path. Wide terminals show the art and roster side by
side, while narrow terminals become roster-only.

States currently derive from normalized messages, reasoning, tool calls,
errors, goals, and preserved input-request events. Codex task start/complete
lifecycle records are still dropped by the normalizer, so the reducer uses
leases and a short ready debounce instead of claiming authoritative runtime
status. The recent-Ready roster is explicitly an observed-run heuristic, not an
authoritative completion log. It includes only work seen while the pet is
running because Relay seeds existing session files from their snapshotted EOF.
Codex commentary messages count as progress rather than completion now that the
normalized message delivery is preserved.

Rendering uses Kitty graphics where available, the Kitty local-file protocol
in iTerm2 3.6+, and a truecolor ANSI half-block fallback. `bun run dev` cycles
through states for art iteration; `bun run check` runs strict TypeScript and
Bun tests. The checked-in Hachiware frames are explicitly prototype-only fan
art and must be replaced before publishing or distributing the project.

## Provider Sources

- Pi reads JSONL from `~/.pi/agent/sessions` unless `--session-dir` is passed.
- Codex reads JSONL from `~/.codex/sessions` and `~/.codex/archived_sessions` unless `--session-dir` is passed.
- OpenCode reads SQLite from `~/.local/share/opencode/opencode.db` unless `--session-dir` is passed.
- OpenCode opens its database with a WAL-aware read-only SQLite URI so active WAL data is visible without application writes; if that cannot open, it falls back to immutable read-only mode. Viewing sessions never runs migrations.
- OpenCode validates the required `session`, `message`, and `part` tables and columns, then detects optional session columns from the actual SQLite schema.
- OpenCode accepts schemas both with and without the optional `session.model` column; it never runs migrations against the user database.

`SessionRef` carries optional `parent_session_id`, `agent_path`,
`agent_nickname`, and `agent_role`. Codex takes owning identity only from the
first valid `session_meta`, because subagent rollouts can contain copied parent
headers. Pi resolves `parentSession` paths to parent IDs, and OpenCode uses its
session `parent_id`.

For Codex, only `parent_thread_id` establishes a subagent relationship.
`forked_from_id` records that a user fork was created from another thread, but
the fork remains a separate root session.

`tokn-session show` defaults to `--scope self`. `--scope tree` discovers
descendants, prints a compact hierarchy, and then renders every session in a
separate section. Tree output is currently pretty-only; self-scoped JSONL keeps
the existing event-only format. Tree discovery uses header-only relationship
scans, including the provider's global roots when the selected session is an
explicit file path. Historical Codex thread-spawn rollouts omit inherited parent
bootstrap history and begin at the explicit trigger-turn boundary. Other
parented Codex sessions, such as guardian work, retain their body from the start.
If an older thread-spawn rollout has no trustworthy boundary, pretty output
warns that its body is unavailable and JSONL output fails instead of attributing
parent work to the child. Tree sections remain separate rather than merging
timestamps into a single timeline.

## Event IR Status

The shared IR is `AgentEvent`.

Persisted Codex rollout wire types live in the standalone
`tokn-codex-protocol` crate. The crate is intentionally decode-oriented:
stable session, response, agent-communication, turn-context, and world-state
fields are typed; volatile subtrees remain JSON values; and unknown tags retain
their original payloads. It does not mirror Codex's internal Rust API.

`tokn-session-codex` uses those local wire types directly. The published
`codex-protocol` dependency is no longer part of the workspace.

Persisted Pi session wire types similarly live in `tokn-pi-protocol`.
Top-level entries, nested message roles, and content blocks all fall back to
lossless unknown values when Pi adds or changes a shape. `tokn-session-pi`
owns their normalization into `AgentEvent`.

OpenCode wire types live in `tokn-opencode-protocol`. Its `v1` module models
the JSON payloads stored in the SQLite `message.data` and `part.data` columns,
while `run` models JSONL from `opencode run --format json`. Both decode through
native-JSON-first wrappers: unknown tags and malformed known variants remain
inspectable instead of preventing the rest of a session from loading. The
OpenCode source crate still owns SQLite queries, relational row identity, and
normalization into `AgentEvent`.

Current event families include:

- `session_started`
- `provider_changed`
- `session_settings_applied`
- `message`
- `reasoning`
- `goal_updated`
- `agent_activity`
- `tool_call`
- `error`
- `unknown`

Messages carry an orthogonal `delivery` field: `commentary`, `final`, or
`unspecified`. Codex preserves the provider's response phase. Pi and OpenCode
assistant text is final because those persisted message records do not expose a
separate commentary channel. Current Codex `final_answer` and legacy `final`
phases both normalize to `final`; user and other messages use `unspecified`.

Tool calls now carry semantic display metadata:

- `tool_kind`: `shell`, `file_read`, `file_write`, `file_edit`, `search`, `web`, `task`, or `unknown`
- `summary`: compact facts for known tool families, such as shell command/exit code or file edit path and rough line counts

Raw `input` and `output` remain in the IR for debugging and provider-native detail.

Reasoning is intentionally flat:

- `text`
- `summary`
- `encrypted_content`
- `signature`

Pretty rendering shows visible reasoning text and summaries, but does not display encrypted reasoning payloads. JSONL preserves encrypted reasoning in the IR.

Codex `event_msg.thread_settings_applied` maps to
`session_settings_applied`. The normalized event exposes a compact settings
snapshot and retains the provider-native snapshot for JSON consumers. Human
rendering intentionally omits permission internals and embedded developer
instructions. The relay updates `SessionContext.cwd` when these settings change
without replacing the session's original project metadata.

Codex `event_msg.sub_agent_activity` maps to `agent_activity`. Its
`agent_thread_id` and `agent_path` identify the target of the activity, so the
IR names them `target_session_id` and `target_agent_path`. Actor identity is
optional and is not inferred from the containing rollout because child files
can include copied parent history. Human output therefore says `interaction
with /root` unless an actor is independently known. The first Codex
`session_meta` owns the rollout; later copied session headers do not replace
the normalizer or relay session identity.

Reusable display formatting lives in `crates/render`. It depends on `core`, not on terminal libraries. The CLI uses it for linear output and the interactive browser uses its `EventDisplay` rows for collapsed summaries and expanded detail.

Pretty rendering also prefers compact semantic tool lines, such as:

```text
shell cargo test #call_abc
edit crates/core/src/agent_event.rs +4 -1 #call_abc
read crates/cli/src/render.rs #call_abc
```

Unknown tools still render their raw input/output so new provider shapes remain discoverable.
Unknown events preserve raw provider-native payloads when available and pretty rendering shows that native payload.

`browse` is the first interactive historical-session view. Without a session id, it opens an alternate-screen session list; Enter opens the selected session. With a session id, it opens the event browser directly. The event browser uses one row per normalized event. Rows are collapsed by default; expanded rows reuse the same per-event pretty rendering as linear output.

Current browser keys:

- `j`/Down and `k`/Up move the selected event row.
- `h` collapses the selected row; `l` expands it.
- Enter/Space toggles expansion.
- In the session list, Enter opens the selected session.
- `z` expands only the selected row.
- `C` collapses all rows.
- `g`/Home and `G`/End jump to the first/last event.
- Ctrl-D/Ctrl-U move by a coarse page.
- In the event browser opened from the session list, `q`/Esc returns to the session list.
- In direct event browsing and the session list, `q`/Esc quits.

## Current Decisions And Edges

- OpenCode shell tools with nonzero `metadata.exit` are marked as errors even when OpenCode records the tool state as completed.
- Tool kind classification and summary extraction live in `crates/core`; provider normalizers should use the shared helpers where possible.
- OpenCode support currently uses the V1 `message` and `part` tables seen in
  local data, not the newer `session_message` projection. The newer table
  exists locally but is empty, and upstream has repeatedly reset its
  projections, so it is not yet treated as an authoritative history source.
- OpenCode V1 message roles, part types, nested tool states, and run-envelope
  types are decoded by `tokn-opencode-protocol`. Unknown and malformed shapes
  preserve their complete native JSON. The adapter retains SQLite row IDs and
  uses a part row ID as the fallback tool-call ID for historical records that
  lack `callID`.
- Pi native JSONL parsing uses `tokn-pi-protocol`. Unknown message roles such
  as historical `bashExecution` records remain visible without preventing the
  rest of the session from loading.
- Pi compaction, branch-summary, extension, label, session-info, leaf, and
  active-tool records are typed at the wire boundary but remain visible
  as native `unknown` events until the display IR has a useful semantic
  mapping.
- Codex native JSONL parsing uses `tokn-codex-protocol`. New rollout and
  response tags retain their native identity and payload for unknown-event
  discovery instead of being erased by an upstream catch-all enum.
- Codex `response_item.agent_message` and legacy
  `inter_agent_communication` records map to `agent_activity` with
  provider-supplied author and recipient paths. Paths remain null when the
  record does not supply them.
- Codex `world_state`, `turn_context`, and
  `inter_agent_communication_metadata` are known control records and are not
  emitted into the display-oriented event stream.
- Codex `event_msg.thread_goal_updated` maps to the visible `goal_updated` IR event.
- Codex `event_msg.thread_settings_applied` is a full effective snapshot, not a
  diff. Repeated applications remain visible in the event stream.
- Timestamps are provider-native strings/numbers today; there is no unified timestamp type yet.
- The CLI help path currently exits through the same error-printing path as other parser errors.

## Print Invocation Status

`create` and `append` have an initial configurable executor path. They do not assume provider binaries are installed. Pass `--executor <launcher>` or set `TOKN_SESSION_<SOURCE>_EXECUTOR`, such as `TOKN_SESSION_OPENCODE_EXECUTOR`.

The executor is only the launcher, equivalent to the provider binary. Provider-specific print-mode arguments are added by the source adapter. For OpenCode, `create` appends `run --format json <prompt>`, so gateway-style commands look like:

```sh
tokn-session create --source opencode --executor "tokn-gateway proxy opencode --npx --" "create a todo app"
```

`append` supports exactly one target:

```sh
tokn-session append --source opencode --executor "tokn-gateway proxy opencode --npx --" --session <session-id> "next turn"
tokn-session append --source opencode --executor "tokn-gateway proxy opencode --npx --" --continue "next turn"
```

Advanced custom executors may include an argv that is exactly `{prompt}`; in that case the executor is treated as the full command and no provider-specific args are appended.

`--cwd <dir>` runs the executor from a specific working directory.

Current limitation: provider output is inherited directly from the child process. The shared `LiveSessionEvent` envelope now exists in `crates/core`, and `crates/render` can pretty-render live events, but the CLI print path does not consume it yet.

OpenCode has the first live-output normalizer: `OpenCodeLiveNormalizer` parses `opencode run --format json` JSONL envelopes into `LiveSessionEvent`. It currently maps `text`, `reasoning`, `tool_use`, and `error` into normalized `AgentEvent`s and preserves other live envelopes such as `step_start` as unknown native events.

## Known Gaps

- No `attach` command yet.
- Codex and Pi have normalization fixtures. OpenCode now has wire-format
  fixtures plus adapter/source regression tests; full SQLite-backed CLI golden
  tests are still missing.
- The relay uses ZeroMQ `PUB/SUB`, which intentionally has no persistence or
  delivery acknowledgement; subscribers that are disconnected can miss events.
- The terminal pet cannot distinguish every runtime state authoritatively until
  provider task lifecycle and interaction events are represented in `AgentEvent`.

## Useful Smokes

GitHub Actions CI runs Rust formatting, workspace check/test, the CLI build,
and all three Bun app check suites on pushes to `main` and pull requests.

```sh
cargo run -p tokn-session-cli -- list --source codex --limit 1
cargo run -p tokn-session-cli -- list --source opencode --limit 1
cargo run -p tokn-session-cli -- show --source opencode <session-id> --format pretty
cargo run -p tokn-session-relay -- stdout
cargo run -p tokn-session-relay -- zeromq
cd apps/discord-pet && bun run check
cd apps/discord-pet && bun run start -- --help
cd apps/discord-pet && bun run login -- --help
cd apps/pet && bun run check
cd apps/pet && bun run start -- --help
cd apps/terminal-pet && bun run check
cd apps/terminal-pet && bun run snapshot
```

## Next Likely Work

- Wire `create`/`append` stdout through provider live normalizers instead of inheriting child stdout directly.
- Decide whether live stream consumption should live in `client` as callbacks/iterators or in the CLI command path.
- Extend provider fixture coverage with OpenCode SQLite normalization.
- Add CLI golden tests for tiny fixture-backed `list` and `show` outputs.
- Add provider-neutral lifecycle and input-request events to the IR, then remove
  the terminal pet's corresponding heuristics.
- Consider splitting stable event IR notes into `docs/event-ir.md` once the IR changes again.
