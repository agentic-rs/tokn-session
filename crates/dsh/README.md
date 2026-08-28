# DSH session source

Read-only historical DeepSeek Harness support for `tokn-session`:

```sh
cargo run -p tokn-session-cli -- list --source dsh
cargo run -p tokn-session-cli -- show --source dsh <session-id> --format pretty
cargo run -p tokn-session-cli -- show --source dsh <session-id> --format jsonl
cargo run -p tokn-session-cli -- show --source dsh <session-id> --scope tree
```

Discovery searches `$DSH_HOME/sessions`, falling back to `~/.dsh/sessions`.
`--session-dir` overrides that root. `show` also accepts an explicit file path,
an exact session ID, or an unambiguous ID prefix. `browse --source dsh` uses
the same reader.

The source reads `session.jsonl` and `session.jsonl.zstd` recursively. Zstandard
decoding is built in (no external `zstd` program) and supports concatenated
frames. Each file read is bounded to its initial byte length. Invalid JSON,
incomplete/corrupt frames, invalid packed runs, and unsupported format versions
produce path-qualified errors; the reader never repairs or writes session data.
Discovery does not follow nested symbolic links. A missing root is an empty list.

The output is a chronological event view, not the reconstructed model context.
Assembled messages take precedence over redundant streamed chunks; unfinished
steps retain text/reasoning deltas. Tool calls and results are correlated by
turn, step, and call ID. Raw JSON arguments are parsed for tool summaries, with
malformed strings retained. Unknown events and content, lifecycle records,
plugin provenance, and surface replacements remain inspectable. Token usage is
retained as native unknown records because the shared IR has no usage event.
Message counts include assembled user/assistant records, not deltas or tools.
Timestamps retain native epoch milliseconds as strings.

Only `origin: "subagent"` headers establish child relationships. Their immutable
`seedLength` excludes inherited parent events; later `session/end-seed` resume
markers do not erase own history. Ordinary forks remain root sessions.

SQLite storage, relay watching, create/append, and input bridges are not yet
implemented. The log format currently supported is version `0`, based on the
pinned `vendor/dsh` reference. Tests use synthetic fixtures; private local logs
are never copied into the repository.
