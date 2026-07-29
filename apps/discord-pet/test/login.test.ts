import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { loadConfig } from "../src/config";
import { login, setupNote } from "../src/login";

const fixtures: string[] = [];

afterEach(async () => {
  await Promise.all(fixtures.splice(0).map((path) => rm(path, {
    recursive: true,
    force: true
  })));
});

describe("login", () => {
  test("shows where to obtain every credential", () => {
    const note = setupNote("/tmp/discord.yaml");
    expect(note).toContain("Developer Portal");
    expect(note).toContain("Developer Mode");
    expect(note).toContain("Copy ID");
    expect(note).toContain("No privileged Discord intents");
    expect(note).toContain("/tmp/discord.yaml");
  });

  test("validates before saving the protected config", async () => {
    const fixture = await temporaryDirectory();
    const path = join(fixture, "discord.yaml");
    const destinations: string[][] = [];
    await login(path, {
      config_exists: async () => false,
      prompter: {
        secret: async () => "secret",
        text: async (label) => label.startsWith("Server") ? "123" : "456",
        confirm: async () => true
      },
      client_factory: () => ({
        async validateDestination(guildId, channelId) {
          destinations.push([guildId, channelId]);
          return "pet";
        }
      })
    });

    expect(destinations).toEqual([["123", "456"]]);
    expect(await loadConfig(path)).toEqual({
      bot_token: "secret",
      guild_id: "123",
      channel_id: "456"
    });
  });
});

async function temporaryDirectory(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "tokn-discord-pet-"));
  fixtures.push(path);
  return path;
}
