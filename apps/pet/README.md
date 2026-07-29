# Pet Supervisor

`@tokn/pet` is the high-level in-process supervisor for local pet workers. It
owns one Relay event stream, evaluates declarative fan-out rules, and forwards
matching `RelayEvent`s into bounded per-worker queues.

Downstreams are async worker objects in the same Bun runtime, not subprocesses.
The supervisor currently supports:

- one interactive terminal worker
- any number of independently configured Discord workers

The Relay executable remains the provider boundary and is the only child
process started by the default source adapter.

## Initial configuration

Copy the checked-in example:

```sh
mkdir -p ~/.tokn/pet
cp pet.example.yaml ~/.tokn/pet/pet.yaml
```

The example routes:

- every Relay event to the terminal worker, preserving its full state model
- root user messages and final assistant messages to Discord only when
  `session.project.repository_name` matches `volty*`

Configure Discord first if needed:

```sh
cd ../discord-pet
bun run login
```

Then run the supervisor:

```sh
cd ../pet
bun install
bun run start
```

Use `q` or Escape in the terminal worker to stop the whole supervisor.

## Configuration

The default file is `~/.tokn/pet/pet.yaml`:

```yaml
version: 1

workers:
  terminal:
    type: terminal

  discord_volty:
    type: discord
    config: ~/.tokn/pet/discord.yaml

rules:
  - forward_to: [terminal]

  - forward_to: [discord_volty]
    when:
      root_only: true
      repository_names: ["volty*"]
      event_types: [message]
      roles: [user]

  - forward_to: [discord_volty]
    when:
      root_only: true
      repository_names: ["volty*"]
      event_types: [message]
      roles: [assistant]
      deliveries: [final]
```

All fields inside `when` are ANDed. Values in one array are ORed. Every
matching rule forwards, and each target receives an event at most once even
when several rules match. An event with no matching rules is dropped.

Supported match fields:

- `providers`
- `event_types`
- `roles`
- `deliveries`
- `repository_names`, with case-insensitive `*` and `?` globs
- `root_only`

Multiple Discord destinations are separate workers:

```yaml
workers:
  terminal:
    type: terminal
  discord_team:
    type: discord
    config: ~/.tokn/pet/discord-team.yaml
  discord_private:
    type: discord
    config: ~/.tokn/pet/discord-private.yaml
```

Each Discord config provides its own credentials, channel, and persistent
thread map. The default `discord.yaml` retains `discord-state.json`; custom
files such as `discord-team.yaml` use `discord-team-state.json`.

## Worker behavior

Each worker has `start`, `handle`, and `stop` lifecycle methods. Events are
processed serially per worker, while different workers progress independently.
Queues are bounded at 256 events by default; when a queue fills, Relay
consumption applies backpressure instead of dropping events. A handler failure
is reported and isolated so later events can still be processed.

Override the queue size with:

```sh
bun run start -- --queue-capacity 512
```

The usual Relay overrides remain available:

```sh
bun run start -- --relay-bin /path/to/tokn-session-relay
bun run start -- --codex-dir /path/to/codex --pi-dir /path/to/pi
bun run start -- --stdin
```

## Verify

```sh
bun run check
bun run start -- --help
```
