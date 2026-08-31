# Tokn Sessions Viewer

The viewer is a read-only Tauri desktop app for browsing historical Pi, Codex,
OpenCode, and DeepSeek Harness (DSH) sessions in one place. It shows root
sessions in a searchable, provider-filterable sidebar and renders their
normalized event streams as conversations with inspectable technical events.
It does not create sessions, append messages, or modify provider data.

## Development

Install the platform prerequisites from the
[Tauri 2 documentation](https://v2.tauri.app/start/prerequisites/), then run:

```sh
cd apps/viewer
pnpm install --frozen-lockfile
pnpm run check
pnpm tauri dev
```

`pnpm run dev` starts only the Vite frontend and is useful for visual work that
does not require native commands. Build the desktop application with:

```sh
pnpm tauri build
```

## Using the viewer

The sidebar discovers root sessions from every provider at startup. Use the
provider pills to include or exclude sources, type in the search box to match a
title, first user prompt, session id, project, or working directory, and select
a row to load its newest normalized events. Rows prefer the provider's native
title, fall back to a preview of the first meaningful user prompt, and otherwise
show **Untitled session**. The shortened id beside the title remains a separate
identity field; the full id is available to assistive technology and on hover.
Earlier history is loaded on demand. Technical event headers expand in place,
while their **Inspect** action opens the full inspector. Messages and reasoning
have a readable **Content** view, while **Normalized** and **Native** expose the
debugging representations.

Known shell, file, search, web, and task tools use compact semantic headers.
Expanding a tool fetches its output lazily, including the matching result when a
provider records invocation and result separately under the same call id. The
inline preview is bounded, selectable plain text or JSON; it never renders as
Markdown or HTML. User and assistant messages, expanded reasoning, and readable
inspector content do render GitHub-flavored Markdown. Raw HTML is disabled,
remote images are never loaded, and links remain inert so provider content
cannot navigate the WebView.

Provider storage is resolved as follows:

- Codex: `$CODEX_HOME`, then the platform home directory's `.codex` folder.
- Pi: `$PI_CODING_AGENT_SESSION_DIR`, `$PI_CODING_AGENT_DIR/sessions`, then the
  platform home directory's `.pi/agent/sessions` folder.
- OpenCode: `$OPENCODE_DB`, including paths relative to
  `$XDG_DATA_HOME/opencode`; otherwise `$XDG_DATA_HOME/opencode/opencode.db` or
  the upstream home-directory fallback is used.
- DSH: `$DSH_HOME/sessions`, then the platform home directory's `.dsh/sessions`
  folder.

Restart the app to discover provider changes made after it opened. This first
version is historical and read-only: it has no composer and never mutates the
provider session files.

## Architecture

React owns the presentation and calls a small set of typed Tauri commands. The
Rust backend invokes the workspace's `client`, `core`, and `render` crates
directly; it does not parse CLI output or depend on the session relay. Provider
histories normalize to `AgentEvent` before source-neutral, snake-case DTOs
cross the IPC boundary. Source errors remain isolated so one unavailable
provider does not hide sessions from the others.

Session discovery starts with provider headers or catalog rows and deliberately
does not compute message or event counts. Indexed native titles are returned in
that cheap pass. Visible untitled rows are then hydrated lazily with a
provider-specific history scan, while a prompt-text search may hydrate
additional candidates to determine whether they match. Hydrated headers are
cached by source revision, and overlapping requests share each session's
in-flight scan. Codex can also
read title metadata from its optional private `state_5.sqlite` in read-only
mode; rows are correlated by both thread id and rollout path, and incompatible
or unavailable private state fails softly. The selected session's normalized
event count arrives with its first event page. Session and event responses are
listed in bounded pages; conversational Markdown previews are capped before IPC,
tool-card fields are also capped, and full event detail is loaded only when
requested. Inline tool output keeps at most 64 KiB using a head-and-tail preview;
the full normalized and native inspector representations retain their separate
512 KiB limits. The backend keeps at most one normalized session snapshot and
reuses it across page and inspector requests while the source revision is
unchanged; OpenCode revision checks include its SQLite WAL and SHM sidecars. The
current provider readers may still load an entire selected session before
producing an event page, so paging does not yet bound parser memory for very
large histories.
Each normalized and provider-native inspector representation is capped at 512
KiB before IPC. Oversized values become structured JSON truncation placeholders;
an uncapped export path is future work.
