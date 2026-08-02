# Provider input extensions

This directory contains optional provider-side bridges for sending user input
to an already-running agent session.

Each provider owns its extension implementation and lifecycle:

- `pi/` contains the Pi session bridge.
- `codex/` is reserved for a future Codex bridge.

The extensions are deliberately separate from the provider implementations and
from Relay. Relay remains the source of truth for observed session events; an
extension only exposes a local input endpoint for the live process that loaded
it.

These extensions are opt-in. They run inside the provider process with that
process's permissions, so only load code you trust.
