import { afterEach, describe, expect, test } from "bun:test";
import {
  chmod,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  loadConfig,
  parseConfig,
  saveConfig,
  statePath
} from "../src/config";

const fixtures: string[] = [];

afterEach(async () => {
  await Promise.all(fixtures.splice(0).map((path) => rm(path, {
    recursive: true,
    force: true
  })));
});

describe("config", () => {
  test("parses a strict config", () => {
    expect(parseConfig({
      bot_token: "secret",
      guild_id: "123",
      channel_id: "456"
    })).toEqual({
      bot_token: "secret",
      guild_id: "123",
      channel_id: "456"
    });
    expect(() => parseConfig({
      bot_token: "secret",
      guild_id: "123",
      channel_id: "456",
      extra: true
    })).toThrow("unknown field");
    expect(() => parseConfig({
      bot_token: "secret",
      guild_id: "server",
      channel_id: "456"
    })).toThrow("must contain only digits");
  });

  test("saves, protects, and reloads YAML", async () => {
    const fixture = await temporaryDirectory();
    const path = join(fixture, "nested", "discord.yaml");
    await saveConfig(path, {
      bot_token: "secret:#token",
      guild_id: "123",
      channel_id: "456",
      bot_username: "session-pet"
    });

    expect(await loadConfig(path)).toEqual({
      bot_token: "secret:#token",
      guild_id: "123",
      channel_id: "456",
      bot_username: "session-pet"
    });
    expect(await readFile(path, "utf8")).toContain("bot_token:");
    expect(await readFile(path, "utf8")).toContain(
      'bot_username: "session-pet"'
    );
    if (process.platform !== "win32") {
      expect((await stat(path)).mode & 0o777).toBe(0o600);
    }
  });

  test("repairs permissions when replacing an existing config", async () => {
    if (process.platform === "win32") {
      return;
    }
    const fixture = await temporaryDirectory();
    const path = join(fixture, "discord.yaml");
    await writeFile(path, "old");
    await chmod(path, 0o644);

    await saveConfig(path, {
      bot_token: "secret",
      guild_id: "123",
      channel_id: "456"
    });

    expect((await stat(path)).mode & 0o777).toBe(0o600);
  });

  test("keeps bot_username optional for older configs", async () => {
    expect(parseConfig({
      bot_token: "secret",
      guild_id: "123",
      channel_id: "456"
    })).toEqual({
      bot_token: "secret",
      guild_id: "123",
      channel_id: "456"
    });
  });

  test("keeps state beside a custom config", () => {
    expect(statePath("/tmp/pet/custom.yaml")).toBe(
      "/tmp/pet/custom-state.json"
    );
    expect(statePath("/tmp/pet/discord.yaml")).toBe(
      "/tmp/pet/discord-state.json"
    );
  });
});

async function temporaryDirectory(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "tokn-discord-pet-"));
  fixtures.push(path);
  return path;
}
