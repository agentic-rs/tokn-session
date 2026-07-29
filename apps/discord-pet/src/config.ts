import { readFile, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, extname, join, resolve } from "node:path";

import { asObject } from "./protocol";
import { writeFileAtomically } from "./storage";

export interface DiscordConfig {
  bot_token: string;
  guild_id: string;
  channel_id: string;
  bot_username?: string;
}

const CONFIG_FIELDS = new Set([
  "bot_token",
  "guild_id",
  "channel_id",
  "bot_username"
]);

export function defaultConfigPath(): string {
  return join(homedir(), ".tokn", "pet", "discord.yaml");
}

export function statePath(configPath: string): string {
  const resolved = resolve(configPath);
  const extension = extname(resolved);
  const stem = basename(resolved, extension);
  const filename = stem === "discord"
    ? "discord-state.json"
    : `${stem}-state.json`;
  return join(dirname(resolved), filename);
}

export async function loadConfig(path: string): Promise<DiscordConfig> {
  let contents: string;
  try {
    contents = await readFile(path, "utf8");
  } catch (error) {
    throw new Error(`failed to read Discord pet config ${path}: ${errorMessage(error)}`);
  }

  let parsed: unknown;
  try {
    parsed = Bun.YAML.parse(contents);
  } catch (error) {
    throw new Error(`failed to parse Discord pet config ${path}: ${errorMessage(error)}`);
  }
  return parseConfig(parsed);
}

export function parseConfig(value: unknown): DiscordConfig {
  const record = asObject(value);
  if (!record) {
    throw new Error("Discord pet config must be a YAML object");
  }
  const unknownFields = Object.keys(record).filter((field) => !CONFIG_FIELDS.has(field));
  if (unknownFields.length > 0) {
    throw new Error(`Discord pet config contains unknown field \`${unknownFields[0]}\``);
  }
  const config: DiscordConfig = {
    bot_token: stringField(record.bot_token, "bot_token"),
    guild_id: stringField(record.guild_id, "guild_id"),
    channel_id: stringField(record.channel_id, "channel_id")
  };
  if (record.bot_username !== undefined) {
    config.bot_username = stringField(record.bot_username, "bot_username");
  }
  validateConfig(config);
  return config;
}

export function validateConfig(config: DiscordConfig): void {
  if (config.bot_token.trim().length === 0) {
    throw new Error("Discord pet config `bot_token` must not be empty");
  }
  validateSnowflake("guild_id", config.guild_id);
  validateSnowflake("channel_id", config.channel_id);
  if (config.bot_username !== undefined && config.bot_username.length === 0) {
    throw new Error("Discord pet config `bot_username` must not be empty");
  }
}

export async function saveConfig(path: string, config: DiscordConfig): Promise<void> {
  validateConfig(config);
  const lines = [
    `bot_token: ${JSON.stringify(config.bot_token)}`,
    `guild_id: ${JSON.stringify(config.guild_id)}`,
    `channel_id: ${JSON.stringify(config.channel_id)}`
  ];
  if (config.bot_username !== undefined) {
    lines.push(`bot_username: ${JSON.stringify(config.bot_username)}`);
  }
  const yaml = [...lines, ""].join("\n");
  await writeFileAtomically(path, yaml);
}

export async function permissionsWarning(path: string): Promise<string | undefined> {
  if (process.platform === "win32") {
    return undefined;
  }
  try {
    const mode = (await stat(path)).mode & 0o077;
    return mode === 0
      ? undefined
      : `Discord config ${path} is accessible by other users; run chmod 600 ${path}`;
  } catch {
    return undefined;
  }
}

function stringField(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new Error(`Discord pet config \`${field}\` must be a string`);
  }
  return value.trim();
}

function validateSnowflake(field: string, value: string): void {
  if (!/^\d+$/.test(value)) {
    throw new Error(`Discord pet config \`${field}\` must contain only digits`);
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
