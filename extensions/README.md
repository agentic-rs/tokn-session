# Provider input extensions

This directory contains optional provider-side bridges for sending user input
to an already-running agent session.

Each provider owns its extension implementation and lifecycle:

- `pi/` contains the Pi session bridge.
- `codex/` is reserved for a future Codex bridge.

Provider wire contracts shared with clients live below the provider directory's
`lib/` folder so Pi does not mistake them for standalone extension entry points.

The extensions are deliberately separate from the provider implementations and
from Relay. Relay remains the source of truth for observed session events; an
extension only exposes a local input endpoint for the live process that loaded
it. Process-scoped transports publish session-scoped descriptors so clients can
resolve an observed session to the process instance that currently owns it.

These extensions are opt-in. They run inside the provider process with that
process's permissions, so only load code you trust.
