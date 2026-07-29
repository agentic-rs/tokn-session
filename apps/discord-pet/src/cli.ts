import { defaultConfigPath } from "./config";

export interface RunCommand {
  kind: "run";
  config: string;
  stdin: boolean;
  relay_bin?: string;
  codex_dir?: string;
  pi_dir?: string;
}

export interface LoginCommand {
  kind: "login";
  config: string;
}

export interface HelpCommand {
  kind: "help";
  command: "root" | "run" | "login";
}

export type Command = RunCommand | LoginCommand | HelpCommand;

export function parseArgs(args: string[]): Command {
  if (args[0] === "login") {
    return parseLogin(args.slice(1));
  }
  if (args[0] === "run") {
    return parseRun(args.slice(1));
  }
  if (["-h", "--help"].includes(args[0] ?? "")) {
    return { kind: "help", command: "root" };
  }
  return parseRun(args);
}

export function help(command: HelpCommand["command"]): string {
  switch (command) {
    case "root":
      return `tokn Discord pet

Usage:
  bun run start
  bun run start -- login
  bun run start -- run [options]

Commands:
  run    Follow Relay events and publish root user/final messages (default)
  login  Interactively configure and validate Discord credentials

Run \`bun run start -- <command> --help\` for command options.`;
    case "run":
      return `Follow Relay events and publish root user/final messages.

Usage:
  bun run start -- run [options]

Options:
  --config <path>     Config file (default: ~/.tokn/pet/discord.yaml)
  --stdin             Read RelayEvent JSONL from stdin
  --relay-bin <path>  Spawn an installed tokn-session-relay binary
  --codex-dir <path>  Override the Codex session root
  --pi-dir <path>     Override the Pi session root
  -h, --help          Show this help`;
    case "login":
      return `Interactively configure and validate Discord credentials.

Usage:
  bun run login
  bun run start -- login [options]

Options:
  --config <path>  Config file (default: ~/.tokn/pet/discord.yaml)
  -h, --help       Show this help`;
  }
}

function parseRun(args: string[]): RunCommand | HelpCommand {
  const command: RunCommand = {
    kind: "run",
    config: defaultConfigPath(),
    stdin: false
  };
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]!;
    switch (argument) {
      case "--config":
        command.config = nextValue(args, ++index, argument);
        break;
      case "--stdin":
        command.stdin = true;
        break;
      case "--relay-bin":
        command.relay_bin = nextValue(args, ++index, argument);
        break;
      case "--codex-dir":
        command.codex_dir = nextValue(args, ++index, argument);
        break;
      case "--pi-dir":
        command.pi_dir = nextValue(args, ++index, argument);
        break;
      case "-h":
      case "--help":
        return { kind: "help", command: "run" };
      default:
        throw new Error(`unknown run option: ${argument}`);
    }
  }
  return command;
}

function parseLogin(args: string[]): LoginCommand | HelpCommand {
  const command: LoginCommand = {
    kind: "login",
    config: defaultConfigPath()
  };
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]!;
    switch (argument) {
      case "--config":
        command.config = nextValue(args, ++index, argument);
        break;
      case "-h":
      case "--help":
        return { kind: "help", command: "login" };
      default:
        throw new Error(`unknown login option: ${argument}`);
    }
  }
  return command;
}

function nextValue(args: string[], index: number, option: string): string {
  const value = args[index];
  if (!value) {
    throw new Error(`${option} requires a value`);
  }
  return value;
}
