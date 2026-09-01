# tokn-session

`tokn-session` is a provider-agnostic session layer for agent tools. It can
discover and normalize historical sessions from Pi, Codex, OpenCode, ZCode,
and DeepSeek Harness (DSH), while preserving provider-native detail needed for
display and debugging.

The Rust CLI currently supports listing, showing, and browsing sessions, plus
the initial configurable create/append path. A relay provides normalized live
events to the terminal and Discord pet applications.

```sh
cargo run -p tokn-session-cli -- list --source codex --limit 5
cargo run -p tokn-session-cli -- show --source pi <session-id>
cargo run -p tokn-session-cli -- browse --source dsh
cargo run -p tokn-session-cli -- list --source zcode --limit 5
```

## Desktop viewer

`apps/viewer` is a read-only Tauri app that presents root sessions from all
five providers in one searchable interface. It reuses the Rust session crates
directly rather than parsing CLI output and safely renders conversational
Markdown without allowing provider content to navigate the WebView. A local,
metadata-only index keeps its sidebar current without writing provider data.

```sh
cd apps/viewer
pnpm install
pnpm run check
pnpm tauri dev
```

See [apps/viewer/README.md](apps/viewer/README.md) for build instructions and
architecture, and [docs/handoff.md](docs/handoff.md) for detailed current
implementation status.
