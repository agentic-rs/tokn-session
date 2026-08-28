import { readFile } from "node:fs/promises";

import type { DiscordMessage } from "./discord";
import { asObject, asString, type RelayEvent } from "./protocol";
import { writeFileAtomically } from "./storage";

const EMBED_DESCRIPTION_LIMIT = 3_900;
const USER_COLOR = 0x5865f2;
const FINAL_COLOR = 0x57f287;

interface DiscordApi {
  createMessage(channelId: string, message: DiscordMessage): Promise<string>;
  createThread(channelId: string, messageId: string, name: string): Promise<string>;
}

interface PetState {
  threads: Record<string, string>;
}

type PublishedKind = "user" | "final";

export class DiscordPet {
  readonly #api: DiscordApi;
  readonly #channelId: string;
  readonly #statePath: string;
  readonly #threads: Record<string, string>;

  private constructor(
    api: DiscordApi,
    channelId: string,
    statePath: string,
    state: PetState
  ) {
    this.#api = api;
    this.#channelId = channelId;
    this.#statePath = statePath;
    this.#threads = state.threads;
  }

  static async create(
    api: DiscordApi,
    channelId: string,
    statePath: string
  ): Promise<DiscordPet> {
    return new DiscordPet(api, channelId, statePath, await loadState(statePath));
  }

  async process(relay: RelayEvent): Promise<void> {
    if (relay.session.parent_session_id) {
      return;
    }
    const kind = publishedKind(relay);
    const text = asString(relay.event.text)?.trim();
    if (!kind || !text) {
      return;
    }

    const chunks = splitMessage(text);
    let threadId = this.#threads[relay.topic];
    let remaining = chunks;
    let hadExistingThread = true;
    if (!threadId) {
      hadExistingThread = false;
      const first = chunks[0];
      if (!first) {
        return;
      }
      const starterId = await this.#api.createMessage(
        this.#channelId,
        discordMessage(kind, first, false)
      );
      threadId = await this.#api.createThread(
        this.#channelId,
        starterId,
        threadName(relay)
      );
      this.#threads[relay.topic] = threadId;
      await saveState(this.#statePath, { threads: this.#threads });
      remaining = chunks.slice(1);
    }

    for (const [index, chunk] of remaining.entries()) {
      await this.#api.createMessage(
        threadId,
        discordMessage(kind, chunk, index > 0 || !hadExistingThread)
      );
    }
  }
}

export function splitMessage(
  text: string,
  maxUtf16Units = EMBED_DESCRIPTION_LIMIT
): string[] {
  const chunks: string[] = [];
  let chunk = "";
  let units = 0;
  for (const character of text) {
    const characterUnits = character.length;
    if (units + characterUnits > maxUtf16Units && chunk.length > 0) {
      chunks.push(chunk);
      chunk = "";
      units = 0;
    }
    chunk += character;
    units += characterUnits;
  }
  if (chunk.length > 0) {
    chunks.push(chunk);
  }
  return chunks;
}

function publishedKind(relay: RelayEvent): PublishedKind | undefined {
  if (
    relay.event.type !== "message"
    || asString(relay.event.phase) !== "finished"
    || asObject(relay.event.provenance)?.display === false
  ) {
    return undefined;
  }
  const role = asString(relay.event.role);
  if (role === "user") {
    return "user";
  }
  return role === "assistant" && asString(relay.event.delivery) === "final"
    ? "final"
    : undefined;
}

function discordMessage(
  kind: PublishedKind,
  description: string,
  continued: boolean
): DiscordMessage {
  const title = kind === "user" ? "User" : "Final";
  return {
    embeds: [{
      title: continued ? `${title} · continued` : title,
      description,
      color: kind === "user" ? USER_COLOR : FINAL_COLOR
    }],
    allowed_mentions: {
      parse: []
    }
  };
}

function threadName(relay: RelayEvent): string {
  const project = relay.session.project;
  const label = project?.project_name
    ?? project?.folder_name
    ?? project?.repository_name
    ?? project?.name
    ?? relay.session.title
    ?? "session";
  const provider = relay.session.provider ?? "unknown";
  const sessionId = relay.session.session_id.slice(0, 8);
  return truncateUtf16(
    `${singleLine(label)} · ${provider}/${sessionId}`,
    100
  );
}

function singleLine(value: string): string {
  return value.split(/\s+/).filter(Boolean).join(" ");
}

function truncateUtf16(value: string, maxUnits: number): string {
  let output = "";
  let units = 0;
  for (const character of value) {
    if (units + character.length > maxUnits) {
      break;
    }
    output += character;
    units += character.length;
  }
  return output;
}

async function loadState(path: string): Promise<PetState> {
  let contents: string;
  try {
    contents = await readFile(path, "utf8");
  } catch (error) {
    if (isNotFound(error)) {
      return { threads: {} };
    }
    throw new Error(`failed to read Discord pet state ${path}: ${errorMessage(error)}`);
  }

  try {
    const record = asObject(JSON.parse(contents));
    const threads = asObject(record?.threads);
    if (!record || !threads) {
      throw new Error("expected an object with a threads map");
    }
    const parsedThreads: Record<string, string> = {};
    for (const [topic, value] of Object.entries(threads)) {
      const threadId = asString(value);
      if (!threadId) {
        throw new Error(`thread mapping for ${topic} is not a string`);
      }
      parsedThreads[topic] = threadId;
    }
    return { threads: parsedThreads };
  } catch (error) {
    throw new Error(`failed to parse Discord pet state ${path}: ${errorMessage(error)}`);
  }
}

async function saveState(path: string, state: PetState): Promise<void> {
  await writeFileAtomically(path, `${JSON.stringify(state, null, 2)}\n`);
}

function isNotFound(error: unknown): boolean {
  return typeof error === "object"
    && error !== null
    && "code" in error
    && error.code === "ENOENT";
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
