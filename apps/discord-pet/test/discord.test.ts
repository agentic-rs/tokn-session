import { describe, expect, test } from "bun:test";

import { DiscordClient } from "../src/discord";

describe("DiscordClient", () => {
  test("validates the bot and destination guild", async () => {
    const requests: Request[] = [];
    const client = new DiscordClient("secret", {
      base_url: "https://discord.test",
      fetch_fn: async (input, init) => {
        requests.push(new Request(input, init));
        const path = new URL(String(input)).pathname;
        return Response.json(
          path.endsWith("/users/@me")
            ? { username: "pet" }
            : { guild_id: "123" }
        );
      }
    });

    expect(await client.validateDestination("123", "456")).toBe("pet");
    expect(requests).toHaveLength(2);
    expect(requests[0]?.headers.get("authorization")).toBe("Bot secret");
  });

  test("rejects a channel from another guild", async () => {
    const client = new DiscordClient("secret", {
      fetch_fn: async (input) => Response.json(
        String(input).endsWith("/users/@me")
          ? { username: "pet" }
          : { guild_id: "other" }
      )
    });

    expect(client.validateDestination("123", "456")).rejects.toThrow(
      "does not belong"
    );
  });

  test("retries rate limits using Discord retry_after", async () => {
    let requests = 0;
    const sleeps: number[] = [];
    const client = new DiscordClient("secret", {
      fetch_fn: async () => {
        requests += 1;
        return requests === 1
          ? Response.json({ retry_after: 0.01 }, { status: 429 })
          : Response.json({ id: "message-1" });
      },
      sleep_fn: async (milliseconds) => {
        sleeps.push(milliseconds);
      }
    });

    expect(await client.createMessage("456", {
      embeds: [],
      allowed_mentions: { parse: [] }
    })).toBe("message-1");
    expect(sleeps).toEqual([10]);
  });
});
