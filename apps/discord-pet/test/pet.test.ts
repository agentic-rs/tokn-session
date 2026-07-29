import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { DiscordMessage } from "../src/discord";
import { DiscordPet, splitMessage } from "../src/pet";
import { relayEvent } from "./fixtures";

const fixtures: string[] = [];

afterEach(async () => {
  await Promise.all(fixtures.splice(0).map((path) => rm(path, {
    recursive: true,
    force: true
  })));
});

describe("DiscordPet", () => {
  test("publishes only root user and final messages", async () => {
    const fixture = await temporaryDirectory();
    const api = new FakeDiscord();
    const pet = await DiscordPet.create(
      api,
      "channel",
      join(fixture, "state.json")
    );

    await pet.process(relayEvent({
      role: "user",
      phase: "finished",
      text: "hello @everyone"
    }));
    await pet.process(relayEvent({
      role: "assistant",
      delivery: "commentary",
      phase: "finished",
      text: "working"
    }));
    await pet.process(relayEvent({
      role: "assistant",
      delivery: "final",
      phase: "finished",
      text: "done"
    }));
    await pet.process(relayEvent({
      role: "assistant",
      delivery: "final",
      phase: "finished",
      text: "child"
    }, {
      session: {
        session_id: "child",
        parent_session_id: "session-12345678"
      }
    }));

    expect(api.threads).toEqual([{
      channel_id: "channel",
      message_id: "message-0",
      name: "tokn-agent · codex/session-"
    }]);
    expect(api.messages.map(({ channel_id }) => channel_id)).toEqual([
      "channel",
      "thread-0"
    ]);
    expect(api.messages[0]?.message.allowed_mentions).toEqual({ parse: [] });
    expect(api.messages[0]?.message.embeds[0]?.description).toBe("hello @everyone");
    expect(api.messages[1]?.message.embeds[0]?.title).toBe("Final");
  });

  test("persists and restores thread mappings", async () => {
    const fixture = await temporaryDirectory();
    const path = join(fixture, "state.json");
    const firstApi = new FakeDiscord();
    const first = await DiscordPet.create(firstApi, "channel", path);
    const user = relayEvent({
      role: "user",
      phase: "finished",
      text: "first"
    });
    await first.process(user);

    const secondApi = new FakeDiscord();
    const second = await DiscordPet.create(
      secondApi,
      "channel",
      path
    );
    await second.process(relayEvent({
      role: "assistant",
      delivery: "final",
      phase: "finished",
      text: "second"
    }));

    expect(secondApi.threads).toHaveLength(0);
    expect(secondApi.messages[0]?.channel_id).toBe("thread-0");
    expect(JSON.parse(await readFile(path, "utf8"))).toEqual({
      threads: {
        "codex.session-12345678": "thread-0"
      }
    });
  });

  test("splits by UTF-16 length without breaking surrogate pairs", () => {
    expect(splitMessage("a😀b", 3)).toEqual(["a😀", "b"]);
  });
});

class FakeDiscord {
  readonly messages: Array<{
    channel_id: string;
    message: DiscordMessage;
  }> = [];
  readonly threads: Array<{
    channel_id: string;
    message_id: string;
    name: string;
  }> = [];

  async createMessage(channelId: string, message: DiscordMessage): Promise<string> {
    const id = `message-${this.messages.length}`;
    this.messages.push({ channel_id: channelId, message });
    return id;
  }

  async createThread(
    channelId: string,
    messageId: string,
    name: string
  ): Promise<string> {
    const id = `thread-${this.threads.length}`;
    this.threads.push({
      channel_id: channelId,
      message_id: messageId,
      name
    });
    return id;
  }
}

async function temporaryDirectory(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "tokn-discord-pet-"));
  fixtures.push(path);
  return path;
}
