import { describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

import { parseConfig } from "../src/config";

describe("pet config", () => {
  test("parses the checked-in volty routing example", async () => {
    const path = resolve(import.meta.dir, "..", "pet.example.yaml");
    const config = parseConfig(Bun.YAML.parse(await readFile(path, "utf8")));

    expect(Object.keys(config.workers)).toEqual([
      "terminal",
      "discord_volty"
    ]);
    expect(config.rules).toHaveLength(3);
  });

  test("supports multiple Discord worker instances", () => {
    const config = parseConfig({
      version: 1,
      workers: {
        terminal: { type: "terminal" },
        discord_team: {
          type: "discord",
          config: "~/.tokn/pet/discord-team.yaml"
        },
        discord_private: {
          type: "discord",
          config: "~/.tokn/pet/discord-private.yaml"
        }
      },
      rules: [{
        forward_to: ["discord_team", "discord_private"]
      }]
    });

    expect(config.workers.discord_team?.type).toBe("discord");
    expect(config.workers.discord_private?.type).toBe("discord");
  });

  test("rejects unknown workers and fields", () => {
    expect(() => parseConfig({
      version: 1,
      workers: {
        terminal: { type: "terminal" }
      },
      rules: [{
        forward_to: ["missing"]
      }]
    })).toThrow("unknown worker");
    expect(() => parseConfig({
      version: 1,
      workers: {
        terminal: {
          type: "terminal",
          command: "subprocess"
        }
      },
      rules: [{
        forward_to: ["terminal"]
      }]
    })).toThrow("unknown field");
  });
});
