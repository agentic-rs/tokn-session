# Discord Pet

`@tokn/discord-pet` is a Bun application that mirrors local root Codex and Pi
conversations into Discord. It consumes the normalized JSONL emitted by
`tokn-session-relay`, creates one public thread per root session, and publishes
only user messages and final assistant messages.

Commentary, reasoning, tool activity, and child sessions are not published.
The app uses Discord's REST API, so it requires no privileged gateway intents.

## Configure

From this directory:

```sh
bun install
bun run login
```

Login first walks through installing the application to the target server and
waits until the bot appears in the server member list. It then explains where
to obtain the bot token, server ID, and channel ID, hides the token while
typing, validates the installed bot and destination through Discord, and writes
`~/.tokn/pet/discord.yaml` with owner-only permissions. It asks before
replacing an existing configuration.

The installation phase uses the Developer Portal's **Installation** page:

1. Enable **Guild Install**.
2. Select **Discord Provided Link**.
3. Add the `bot` scope to the Guild Install defaults.
4. Grant the permissions listed below and save the changes.
5. Open the install link, choose **Add to server**, and select your server.

The resulting file has this shape:

```yaml
bot_token: "replace-with-the-bot-token"
guild_id: "123456789012345678"
channel_id: "234567890123456789"
```

Use a custom location when needed:

```sh
bun run login -- --config /path/to/discord.yaml
```

The bot needs these permissions in the target text channel:

- View Channel
- Send Messages
- Create Public Threads
- Send Messages in Threads
- Read Message History
- Embed Links

## Run

```sh
bun run start
```

The app builds the workspace Relay once if needed, then spawns:

```sh
tokn-session-relay stdout --format json
```

To consume an existing JSONL pipeline:

```sh
cargo run -q -p tokn-session-relay -- stdout --format json \
  | bun run start -- --stdin
```

Other useful overrides:

```sh
bun run start -- --relay-bin /path/to/tokn-session-relay
bun run start -- --codex-dir /path/to/codex --pi-dir /path/to/pi
bun run start -- --config /path/to/discord.yaml
```

Thread mappings are stored beside the configuration in `discord-state.json`,
so later turns continue in the same Discord thread after a restart. Long
messages are split using Discord's UTF-16 limits, all mentions are disabled,
and transient API errors and rate limits are retried.

## Verify

```sh
bun run check
bun run start -- --help
bun run login -- --help
```
