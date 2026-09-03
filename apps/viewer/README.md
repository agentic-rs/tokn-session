# Tokn Sessions Viewer

The viewer is a read-only desktop and browser app for browsing historical Pi, Codex,
OpenCode, ZCode, WorkBuddy, and DeepSeek Harness (DSH) sessions in one place. It
shows root sessions in a searchable, provider-filterable sidebar, expands their
known subagents on demand, and renders each selected normalized event stream as
a conversation with inspectable technical events.
It does not create sessions, append messages, or modify provider data.

## Relay lifetime and data modes

By default the viewer starts **Automatic Relay** for its lifetime. It runs the
Relay live feed in a headless child of the bundled viewer
executable, over stdio. Shared `viewer-core` owns authoritative snapshots. Development and packaged
builds need no separate Relay installation. The child stops when the app exits,
including parent crashes; external Relay processes are never terminated.

The Relay panel offers Automatic, External, and Local history only modes.
Automatic has optional native Inspector records (off by default). External
connects to an independently configured `tokn-viewer-api snapshot` endpoint.
Local explicitly clears Relay snapshots and resumes direct provider history.
Automatic mode covers all six providers. Automatic
uses the same provider-root environment overrides listed below as local reads.

Startup/status is visible as Starting, Connecting, Live, Retrying, or Failed.
Each child gets ten seconds to report readiness; up to three launch attempts
use one-/two-second backoff. Retry starts a new attempt budget. Failures retain
last-good snapshots without local fallback. Settings persist in app-config
`relay.json`; legacy enabled connections keep their external endpoint, and
legacy explicit disabled settings remain Local. Missing settings default to
Automatic. Relay-backed sessions do not yet have durable unread tracking.

## Development

Install the platform prerequisites from the
[Tauri 2 documentation](https://v2.tauri.app/start/prerequisites/), then run:

```sh
cd apps/viewer
pnpm install --frozen-lockfile
pnpm run check
pnpm tauri dev
```

`pnpm run dev` starts the browser frontend for development. Start `cargo run -p
tokn-viewer-api -- --allow-origin http://localhost:1437` separately, then
connect to its address. For the integrated browser viewer, run `pnpm run build`
and start `tokn-viewer-api` from the repository root; it serves `dist/` and the
API at `http://127.0.0.1:5558`. See [remote setup and API
contract](../../docs/viewer-api.md). Build the desktop application with:

```sh
pnpm tauri build
```

## Using the viewer

For Relay-backed providers the sidebar reads Relay's metadata catalog. For
local providers it starts from the local session index rather than reading
provider storage in the initial viewport. On a fresh database, it shows an indexing
state until the background cataloger has committed each selected provider. Use
the provider pills to include or exclude sources, type in the search box to
match indexed title, preview, session id, project, working directory, or agent
identity, and select a row to load its newest normalized events. A disclosure
control loads that session's direct subagents as indexed metadata only; nested
controls continue the tree, and selecting a child opens the child's independent
timeline. This does not merge child events into the parent conversation or
infer live completion states. Rows prefer the indexed provider title, then an
indexed preview of the first meaningful user prompt, then an agent label for
known subagents, and otherwise show **Untitled session**. The shortened id
beside the title remains a separate identity field; the full id is available to
assistive technology and on hover.
Historical agent-activity records become delegation cards when their native
target id resolves to a canonical direct child of the selected session within
the same provider. **Open** selects that child and makes it available in the
sidebar; unavailable, ambiguous, or non-child targets remain inspectable but
are never guessed. These cards describe recorded activity, not live state.
Earlier history is loaded on demand. Technical event headers expand in place,
while their **Inspect** action opens the full inspector. Messages and reasoning
have a readable **Content** view, while **Normalized** and **Native** expose the
debugging representations.

User prompts and final assistant replies remain in the outer conversation.
Contiguous stretches of intermediate assistant progress and non-message work
are grouped into a compact work trajectory item, keeping the
surrounding conversation easy to scan. A non-final assistant message itself is
enough to make a stretch fold; metadata-only stretches remain flat. Terminal
bookkeeping after a final reply remains as ordinary chronological rows instead
of creating a second **Worked** item. It shows
**Working for …** with a ticking elapsed time when a provider turn-start is
observed, auto-expanding to show the newest work. Completion collapses it to
**Worked for …**; it can be reopened manually. Providers without reliable turn
signals show neutral **Work** until a final reply or turn boundary closes the
run. Durations use provider timestamps, never file modification time.
Expanding it fetches its contained event rows in bounded pages and shows
them as their normal event cards. Each row remains independently inspectable,
and a recorded delegation can still open its verified direct child session.

Relay batches refresh existing timeline and expanded child items without
discarding loaded history. Scrolling follows new work only at the bottom;
reading older rows preserves the scroll anchor. Local indexing also refreshes
the selected session after successful body updates, including progress that
does not qualify for unread attention.

Known shell, file, search, web, and task tools use compact semantic headers.
Expanding a tool fetches its output lazily, including the matching result when a
provider records invocation and result separately under the same call id. The
inline preview is bounded, selectable plain text or JSON; it never renders as
Markdown or HTML. User and assistant messages, expanded reasoning, and readable
inspector content do render GitHub-flavored Markdown. Raw HTML is disabled,
remote images are never loaded, and links remain inert so provider content
cannot navigate the WebView.

Usage events have an inline token card. It labels whether a row is a model call,
operation total, or cumulative session snapshot; snapshots replace earlier
snapshots and must not be summed. Cache counts are already part of input, and
reasoning or total counters remain provider-reported. Reasoning cards use a
safe, single-line preview and load readable Markdown only after expansion.
Encrypted or provider-redacted reasoning stays opaque in the timeline.

Provider storage is resolved as follows:

- Codex: `$CODEX_HOME`, then the platform home directory's `.codex` folder.
- Pi: `$PI_CODING_AGENT_SESSION_DIR`, `$PI_CODING_AGENT_DIR/sessions`, then the
  platform home directory's `.pi/agent/sessions` folder.
- OpenCode: `$OPENCODE_DB`, including paths relative to
  `$XDG_DATA_HOME/opencode`; otherwise `$XDG_DATA_HOME/opencode/opencode.db` or
  the upstream home-directory fallback is used.
- ZCode: `$ZCODE_STORAGE_DIR/cli/db/db.sqlite`, then the platform home
  directory's `.zcode/cli/db/db.sqlite` path.
- WorkBuddy: `$WORKBUDDY_CONFIG_DIR/workbuddy.db`, then
  `$CODEBUDDY_CONFIG_DIR/workbuddy.db`, then the platform home directory's
  `.workbuddy-ai/workbuddy.db` catalog and its `projects` histories.
- DSH: `$DSH_HOME/sessions`, then the platform home directory's `.dsh/sessions`
  folder.

For providers using local history, the app commits a stable provider-header catalog to its shared index at
`~/.tokn/sessions/index.sqlite`, then uses only that durable index for sidebar,
search, and subagent-tree requests. Provider reads at startup belong to the
background cataloger, not those UI requests. It then backfills event-derived
attention and any body-derived title/preview metadata in bounded, newest-first
batches. Codex and Pi JSONL rollouts update from native filesystem
notifications; ordinary writes inspect only the changed source. OpenCode,
ZCode, WorkBuddy, and DSH retain a ten-second provider-local catalog cadence,
while an all-provider catalog runs only at startup, on recovery, on explicit
Retry, and as a five-minute safety sweep. While body work is pending, a
body-only pass runs every second without rediscovering a provider catalog. If
active source membership changes during a catalog pass, the previous catalog
remains visible and the app quietly retries; mutable titles, previews, and
modification times do not become false provider-read errors. A
row has no dot until its body has finished
backfilling, except that a relocated row retains an already-unread dot while
its new path is validated. Sessions first discovered after a provider catalog
exists can become unread only after that body confirmation finds a new unhidden
user message or final assistant reply. A dot on a collapsed parent can represent
unread activity in a known subagent. The open timeline refreshes after successful
body updates, including progress that does not qualify as unread activity.
This version remains read-only: it has no composer and never mutates provider
session files.

## Architecture

React owns the presentation and calls a small set of typed Tauri commands. The
Rust backend invokes the workspace's `client`, `core`, and `render` crates
directly for local providers; Relay-backed providers use received snapshots.
It does not parse CLI output. Provider histories normalize to `AgentEvent`
before source-neutral, snake-case DTOs
cross the IPC boundary. Source errors remain isolated so one unavailable
provider does not hide sessions from the others.

Local session discovery starts with provider headers in the background and
deliberately does not compute message or event counts. A complete header
catalog is committed promptly to the shared SQLite sidebar index at
`~/.tokn/sessions/index.sqlite`; sidebar, search, and tree queries are strictly
index-only. Before a provider's complete-catalog sentinel exists, its rows are
reported as indexing rather than read directly from that provider. The separate
body pass backfills attention and missing title/preview metadata in bounded
newest-first batches; the one-second pending worker reads only those selected
bodies. The all-provider catalog is a recovery safety sweep, while unwatched
providers are discovered on their independent ten-second cadence. The
index retains opaque source checkpoints, session identity/paths, bounded
title/preview/cwd/timestamp/relationship metadata, and unread revisions, but
never event records, native payloads, reasoning, tool I/O, or full message
bodies. Header-only metadata changes update the index without a body replay.
Catalog and body replacements are staged with optimistic source-cursor checks:
a body result must still match the catalog snapshot before it can apply.
Checkpoints include an index-owned mutation generation as well as the
provider-owned cursor, so a same-cursor metadata update cannot be overwritten
by a second viewer. The initial main pane intentionally has no selected
timeline; only an explicit session selection reads provider history. Codex can
also
read title metadata from its optional private `state_5.sqlite` in read-only
mode; rows are correlated by both thread id and rollout path, and incompatible
or unavailable private state fails softly. Known Codex subagents intentionally
ignore title and preview fields inherited from their parent in that private
state; their agent nickname, role, or path becomes the row label instead. The
selected session's normalized
event count arrives with its first event page. Root, direct-subagent, and event
responses are listed in bounded pages. Direct-subagent pages resolve edges only
within one provider, canonicalize duplicate provider IDs by newest provider
timestamp (then path), and keep missing-parent or cyclic records visible rather
than hiding them.

The index keeps source identity normalized in `sources`. Its read-only
`indexed_sessions` SQLite view joins `provider` and `source_key` onto every
session row for diagnostics without duplicating those fields in storage.
Conversational Markdown previews are capped before IPC,
tool-card fields are also capped, and full event detail is loaded only when
requested. Inline tool output keeps at most 64 KiB using a head-and-tail preview;
the full normalized and native inspector representations retain their separate
512 KiB limits. The backend keeps at most one normalized session snapshot and
reuses it across page and inspector requests while the source revision is
unchanged; OpenCode and ZCode revision checks include their SQLite WAL
sidecars.
The independent durable index checkpoint uses the database and WAL but excludes
the reader-writable SHM file. The current provider readers may still load an
entire selected session before producing an event page, so paging does not yet
bound parser memory for very large histories.
Trajectory items are a viewer presentation projection over that normalized
timeline: user prompts, final assistant replies, their terminal bookkeeping,
and hidden-event boundaries remain outside the item, while intermediate
assistant progress is folded into it. The item's inner rows are loaded only
after expansion. Their page is bounded separately from the outer conversation
page, so a long turn cannot make initial timeline loading unbounded.
Each normalized and provider-native inspector representation is capped at 512
KiB before IPC. Oversized values become structured JSON truncation placeholders;
an uncapped export path is future work.

Usage-card counters cross IPC as decimal strings so every Rust `u64` remains
exact in the JavaScript renderer. Reasoning-card summaries contain only safe
preview metadata; encrypted content and signatures stay out of that projection.
