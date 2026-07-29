const DISCORD_API_BASE = "https://discord.com/api/v10";
const MAX_ATTEMPTS = 4;

export interface DiscordEmbed {
  title: string;
  description: string;
  color: number;
}

export interface DiscordMessage {
  embeds: DiscordEmbed[];
  allowed_mentions: {
    parse: string[];
  };
}

interface DiscordClientOptions {
  base_url?: string;
  fetch_fn?: FetchFn;
  sleep_fn?: (milliseconds: number) => Promise<void>;
}

type FetchFn = (input: string, init?: RequestInit) => Promise<Response>;

export class DiscordClient {
  readonly #botToken: string;
  readonly #baseUrl: string;
  readonly #fetch: FetchFn;
  readonly #sleep: (milliseconds: number) => Promise<void>;

  constructor(botToken: string, options: DiscordClientOptions = {}) {
    if (/[\r\n]/.test(botToken)) {
      throw new Error("Discord bot token contains invalid HTTP header characters");
    }
    this.#botToken = botToken;
    this.#baseUrl = options.base_url ?? DISCORD_API_BASE;
    this.#fetch = options.fetch_fn ?? ((input, init) => fetch(input, init));
    this.#sleep = options.sleep_fn ?? Bun.sleep;
  }

  async validateDestination(guildId: string, channelId: string): Promise<string> {
    const user = await this.#request("GET", "/users/@me");
    const channel = await this.#request("GET", `/channels/${channelId}`);
    if (stringValue(channel.guild_id) !== guildId) {
      throw new Error(`Discord channel ${channelId} does not belong to configured guild ${guildId}`);
    }
    return stringValue(user.username) ?? "unknown bot";
  }

  async createMessage(channelId: string, message: DiscordMessage): Promise<string> {
    const response = await this.#request(
      "POST",
      `/channels/${channelId}/messages`,
      message
    );
    const id = stringValue(response.id);
    if (!id) {
      throw new Error("Discord create-message response did not contain an id");
    }
    return id;
  }

  async createThread(channelId: string, messageId: string, name: string): Promise<string> {
    const response = await this.#request(
      "POST",
      `/channels/${channelId}/messages/${messageId}/threads`,
      {
        name,
        auto_archive_duration: 1440
      }
    );
    const id = stringValue(response.id);
    if (!id) {
      throw new Error("Discord create-thread response did not contain an id");
    }
    return id;
  }

  async #request(
    method: string,
    path: string,
    body?: unknown
  ): Promise<Record<string, unknown>> {
    for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt += 1) {
      let response: Response;
      try {
        response = await this.#fetch(`${this.#baseUrl}${path}`, {
          method,
          headers: {
            authorization: `Bot ${this.#botToken}`,
            "content-type": "application/json",
            "user-agent": "@tokn/discord-pet"
          },
          ...(body === undefined ? {} : { body: JSON.stringify(body) })
        });
      } catch (error) {
        if (attempt + 1 < MAX_ATTEMPTS) {
          await this.#sleep(retryDelay(attempt));
          continue;
        }
        throw new Error(`Discord API request failed: ${errorMessage(error)}`);
      }

      const responseBody = await response.text();
      if (response.ok) {
        try {
          const value: unknown = JSON.parse(responseBody);
          if (typeof value === "object" && value !== null && !Array.isArray(value)) {
            return value as Record<string, unknown>;
          }
        } catch {
          // The error below gives callers a stable diagnostic.
        }
        throw new Error("Discord API returned invalid JSON");
      }

      if (response.status === 429 && attempt + 1 < MAX_ATTEMPTS) {
        await this.#sleep(retryAfter(responseBody, attempt));
        continue;
      }
      if (response.status >= 500 && attempt + 1 < MAX_ATTEMPTS) {
        await this.#sleep(retryDelay(attempt));
        continue;
      }
      throw discordError(response.status, responseBody);
    }
    throw new Error("Discord API request exhausted its retry budget");
  }
}

function retryAfter(body: string, attempt: number): number {
  try {
    const parsed: unknown = JSON.parse(body);
    if (typeof parsed === "object" && parsed !== null && "retry_after" in parsed) {
      const seconds = (parsed as { retry_after?: unknown }).retry_after;
      if (typeof seconds === "number" && Number.isFinite(seconds) && seconds >= 0) {
        return Math.min(seconds * 1_000, 60_000);
      }
    }
  } catch {
    // Fall through to exponential backoff.
  }
  return retryDelay(attempt);
}

function retryDelay(attempt: number): number {
  return 250 * (2 ** Math.min(attempt, 4));
}

function discordError(status: number, body: string): Error {
  let message = "unknown Discord API error";
  let code: number | undefined;
  try {
    const parsed: unknown = JSON.parse(body);
    if (typeof parsed === "object" && parsed !== null) {
      const record = parsed as Record<string, unknown>;
      message = stringValue(record.message) ?? message;
      code = typeof record.code === "number" ? record.code : undefined;
    }
  } catch {
    // Use the stable fallback above.
  }
  return new Error(
    `Discord API returned ${status}${code === undefined ? "" : ` code ${code}`}: ${message}`
  );
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
