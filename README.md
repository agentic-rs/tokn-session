# tokn-session

`tokn-session` is a provider-agnostic session layer for agent tools. It can
discover and normalize historical sessions from Pi, Codex, OpenCode, ZCode,
WorkBuddy, and DeepSeek Harness (DSH), while preserving provider-native detail
needed for display and debugging.

The Rust CLI currently supports listing, showing, and browsing sessions, plus
the initial configurable create/append path. A relay provides normalized live
events to the terminal and Discord pet applications.

```sh
cargo run -p tokn-session-cli -- list --source codex --limit 5
cargo run -p tokn-session-cli -- show --source pi <session-id>
cargo run -p tokn-session-cli -- browse --source dsh
cargo run -p tokn-session-cli -- list --source zcode --limit 5
cargo run -p tokn-session-cli -- list --source workbuddy --limit 5
```

## Desktop viewer

`apps/viewer` is a Tauri and browser app that presents root sessions from all
six providers in one searchable interface. It reuses the Rust session crates
directly rather than parsing CLI output and safely renders conversational
Markdown without allowing provider content to navigate the WebView. A local,
metadata-only index keeps its sidebar current without writing provider data.
Its message composer can send to a root Codex task in Codex Desktop or a live
Pi session running the input bridge.

```sh
cd apps/viewer
pnpm install
pnpm run check
pnpm tauri dev
```

See [apps/viewer/README.md](apps/viewer/README.md) for build instructions and
architecture, and [docs/handoff.md](docs/handoff.md) for detailed current
implementation status.

## Remote hosts through a Hub

`tokn-session-hub` provides one browser endpoint for several hosts, with passkey
login, explicit host enrollment, and outbound connections to each host's
`viewer-api`. Hosts allow viewing by default; agent input requires explicit
control access. See [the Hub guide](docs/hub.md) for local setup, HTTPS
configuration, and the trusted-Hub security model.
