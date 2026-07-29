import { defaultConfigPath } from "./config";

export interface Options {
  config: string;
  stdin: boolean;
  relay_bin?: string;
  codex_dir?: string;
  pi_dir?: string;
  queue_capacity?: number;
}

export type ParseResult =
  | { kind: "run"; options: Options }
  | { kind: "help" };

export function parseArgs(args: string[]): ParseResult {
  const options: Options = {
    config: defaultConfigPath(),
    stdin: false
  };
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]!;
    switch (argument) {
      case "--config":
        options.config = nextValue(args, ++index, argument);
        break;
      case "--stdin":
        options.stdin = true;
        break;
      case "--relay-bin":
        options.relay_bin = nextValue(args, ++index, argument);
        break;
      case "--codex-dir":
        options.codex_dir = nextValue(args, ++index, argument);
        break;
      case "--pi-dir":
        options.pi_dir = nextValue(args, ++index, argument);
        break;
      case "--queue-capacity": {
        const value = Number(nextValue(args, ++index, argument));
        if (!Number.isInteger(value) || value < 1) {
          throw new Error("--queue-capacity must be a positive integer");
        }
        options.queue_capacity = value;
        break;
      }
      case "-h":
      case "--help":
        return { kind: "help" };
      default:
        throw new Error(`unknown option: ${argument}`);
    }
  }
  return { kind: "run", options };
}

export function help(): string {
  return `tokn pet supervisor

Usage:
  bun run start [options]

Options:
  --config <path>         Router config (default: ~/.tokn/pet/pet.yaml)
  --stdin                 Read RelayEvent JSONL from stdin
  --relay-bin <path>      Spawn an installed tokn-session-relay binary
  --codex-dir <path>      Override the Codex session root
  --pi-dir <path>         Override the Pi session root
  --queue-capacity <n>    Per-worker bounded queue size (default: 256)
  -h, --help              Show this help`;
}

function nextValue(args: string[], index: number, option: string): string {
  const value = args[index];
  if (!value) {
    throw new Error(`${option} requires a value`);
  }
  return value;
}
