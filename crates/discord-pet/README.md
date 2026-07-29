# Discord Pet

`tokn-discord-pet` follows local Codex and Pi session files and mirrors root
session conversations into Discord:

- the first root user message creates a public thread
- later root user messages continue in the same thread
- only final assistant messages are published
- commentary, reasoning, tools, and child-session messages are ignored

Existing session bodies are not replayed when the pet starts. A newly created
session replays a small tail so its first user message is not missed.

## Configuration

Run the interactive login:

```sh
cargo run -p tokn-discord-pet -- login
```

It explains where to obtain each value, hides the bot token while typing,
validates the bot and destination with Discord, and saves the configuration
with owner-only permissions. If the file already exists, login asks before
replacing it.

The resulting `~/.tokn/pet/discord.yaml` has this shape:

```yaml
bot_token: "replace-with-the-bot-token"
guild_id: "123456789012345678"
channel_id: "123456789012345678"
```

Protect the token:

```sh
chmod 600 ~/.tokn/pet/discord.yaml
```

Manual creation remains supported, but `login` is preferred because invalid
credentials are rejected before anything is saved. Use
`tokn-discord-pet login --config /path/to/discord.yaml` to configure a
non-default path.

The configured bot needs these permissions in the target text channel:

- View Channel
- Send Messages
- Create Public Threads
- Send Messages in Threads
- Read Message History
- Embed Links

The pet validates the bot token and channel/guild pairing before following
sessions. It does not require privileged Discord intents.

Session-to-thread mappings are stored in
`~/.tokn/pet/discord-state.json`. Keep only one pet process running against a
configuration.

## Run

```sh
cargo run -p tokn-discord-pet
```

For development or tests, paths can be overridden:

```sh
cargo run -p tokn-discord-pet -- \
  --config /path/to/discord.yaml \
  --codex-dir /path/to/codex/sessions \
  --pi-dir /path/to/pi/sessions
```
