import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";

export interface TerminalWorkerConfig {
  type: "terminal";
  color?: boolean;
  name?: string;
  protocol?: "auto" | "ansi" | "kitty" | "kitty_file";
}

export interface DiscordWorkerConfig {
  type: "discord";
  config: string;
}

export type WorkerConfig = TerminalWorkerConfig | DiscordWorkerConfig;

export interface MatchConfig {
  providers?: string[];
  event_types?: string[];
  roles?: string[];
  deliveries?: string[];
  repository_names?: string[];
  root_only?: boolean;
}

export interface RouteConfig {
  forward_to: string[];
  when?: MatchConfig;
}

export interface PetConfig {
  version: 1;
  workers: Record<string, WorkerConfig>;
  rules: RouteConfig[];
}

const ROOT_FIELDS = new Set(["version", "workers", "rules"]);
const TERMINAL_FIELDS = new Set(["type", "color", "name", "protocol"]);
const DISCORD_FIELDS = new Set(["type", "config"]);
const ROUTE_FIELDS = new Set(["forward_to", "when"]);
const MATCH_FIELDS = new Set([
  "providers",
  "event_types",
  "roles",
  "deliveries",
  "repository_names",
  "root_only"
]);
const PROTOCOLS = new Set(["auto", "ansi", "kitty", "kitty_file"]);

export function defaultConfigPath(): string {
  return join(homedir(), ".tokn", "pet", "pet.yaml");
}

export async function loadConfig(path: string): Promise<PetConfig> {
  let contents: string;
  try {
    contents = await readFile(path, "utf8");
  } catch (error) {
    throw new Error(`failed to read pet config ${path}: ${errorMessage(error)}`);
  }
  try {
    return parseConfig(Bun.YAML.parse(contents));
  } catch (error) {
    throw new Error(`failed to parse pet config ${path}: ${errorMessage(error)}`);
  }
}

export function parseConfig(value: unknown): PetConfig {
  const root = objectValue(value, "pet config");
  rejectUnknownFields(root, ROOT_FIELDS, "pet config");
  if (root.version !== 1) {
    throw new Error("pet config `version` must be 1");
  }

  const workerRecords = objectValue(root.workers, "pet config `workers`");
  const workers: Record<string, WorkerConfig> = {};
  for (const [name, workerValue] of Object.entries(workerRecords)) {
    if (!/^[a-zA-Z0-9_-]+$/.test(name)) {
      throw new Error(`pet worker name \`${name}\` contains unsupported characters`);
    }
    workers[name] = parseWorker(name, workerValue);
  }
  if (Object.keys(workers).length === 0) {
    throw new Error("pet config must define at least one worker");
  }

  if (!Array.isArray(root.rules) || root.rules.length === 0) {
    throw new Error("pet config `rules` must be a non-empty array");
  }
  const rules = root.rules.map((rule, index) => parseRoute(rule, index, workers));
  return {
    version: 1,
    workers,
    rules
  };
}

export function expandHome(path: string): string {
  if (path === "~") {
    return homedir();
  }
  if (path.startsWith("~/")) {
    return join(homedir(), path.slice(2));
  }
  return resolve(path);
}

function parseWorker(name: string, value: unknown): WorkerConfig {
  const worker = objectValue(value, `pet worker \`${name}\``);
  if (worker.type === "terminal") {
    rejectUnknownFields(worker, TERMINAL_FIELDS, `terminal worker \`${name}\``);
    const parsed: TerminalWorkerConfig = { type: "terminal" };
    if (worker.color !== undefined) {
      parsed.color = booleanValue(worker.color, `${name}.color`);
    }
    if (worker.name !== undefined) {
      parsed.name = stringValue(worker.name, `${name}.name`);
    }
    if (worker.protocol !== undefined) {
      const protocol = stringValue(worker.protocol, `${name}.protocol`);
      if (!PROTOCOLS.has(protocol)) {
        throw new Error(`${name}.protocol must be auto, ansi, kitty, or kitty_file`);
      }
      parsed.protocol = protocol as TerminalWorkerConfig["protocol"];
    }
    return parsed;
  }
  if (worker.type === "discord") {
    rejectUnknownFields(worker, DISCORD_FIELDS, `Discord worker \`${name}\``);
    return {
      type: "discord",
      config: stringValue(worker.config, `${name}.config`)
    };
  }
  throw new Error(`pet worker \`${name}\` has unsupported type`);
}

function parseRoute(
  value: unknown,
  index: number,
  workers: Record<string, WorkerConfig>
): RouteConfig {
  const route = objectValue(value, `pet rule ${index}`);
  rejectUnknownFields(route, ROUTE_FIELDS, `pet rule ${index}`);
  const forwardTo = stringArray(route.forward_to, `rules[${index}].forward_to`);
  for (const target of forwardTo) {
    if (!workers[target]) {
      throw new Error(`pet rule ${index} targets unknown worker \`${target}\``);
    }
  }
  const parsed: RouteConfig = { forward_to: forwardTo };
  if (route.when !== undefined) {
    parsed.when = parseMatch(route.when, index);
  }
  return parsed;
}

function parseMatch(value: unknown, index: number): MatchConfig {
  const match = objectValue(value, `rules[${index}].when`);
  rejectUnknownFields(match, MATCH_FIELDS, `rules[${index}].when`);
  const parsed: MatchConfig = {};
  for (const field of [
    "providers",
    "event_types",
    "roles",
    "deliveries",
    "repository_names"
  ] as const) {
    if (match[field] !== undefined) {
      parsed[field] = stringArray(match[field], `rules[${index}].when.${field}`);
    }
  }
  if (match.root_only !== undefined) {
    parsed.root_only = booleanValue(
      match.root_only,
      `rules[${index}].when.root_only`
    );
  }
  return parsed;
}

function objectValue(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

function stringValue(value: unknown, label: string): string {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value.trim();
}

function booleanValue(value: unknown, label: string): boolean {
  if (typeof value !== "boolean") {
    throw new Error(`${label} must be a boolean`);
  }
  return value;
}

function stringArray(value: unknown, label: string): string[] {
  if (!Array.isArray(value) || value.length === 0) {
    throw new Error(`${label} must be a non-empty string array`);
  }
  return value.map((item) => stringValue(item, label));
}

function rejectUnknownFields(
  value: Record<string, unknown>,
  allowed: Set<string>,
  label: string
): void {
  const unknown = Object.keys(value).find((field) => !allowed.has(field));
  if (unknown) {
    throw new Error(`${label} contains unknown field \`${unknown}\``);
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
