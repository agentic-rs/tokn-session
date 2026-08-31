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
session id or working directory, and select a row to load its newest normalized
events. Earlier history is loaded on demand. Select any message or technical
event to open the inspector; messages and reasoning have a readable **Content**
view, while **Normalized** and **Native** expose the debugging representations.

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

Session discovery reads provider headers or catalog rows only; it deliberately
does not compute message or event counts. The selected session's normalized
event count arrives with its first event page. Session and event responses are
paged to bound IPC payloads, and full event detail is loaded only when
requested. The backend keeps at most one normalized session snapshot and reuses
it across page and inspector requests while the source revision is unchanged;
OpenCode revision checks include its SQLite WAL and SHM sidecars. The current
provider readers may still load an entire selected session before producing an
event page, so paging does not yet bound parser memory for very large histories.
Each normalized and provider-native inspector representation is capped at 512
KiB before IPC. Oversized values become structured JSON truncation placeholders;
an uncapped export path is future work.
