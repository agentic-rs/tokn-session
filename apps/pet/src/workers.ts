import {
  loadConfig as loadDiscordConfig,
  permissionsWarning,
  statePath
} from "@tokn/discord-pet/config";
import { DiscordClient } from "@tokn/discord-pet/discord";
import { DiscordPet } from "@tokn/discord-pet/pet";
import type { RelayEvent } from "@tokn/discord-pet/protocol";
import { TerminalPetWorker } from "@tokn/terminal-pet/worker";

import {
  expandHome,
  type DiscordWorkerConfig,
  type TerminalWorkerConfig,
  type WorkerConfig
} from "./config";
import type { PetWorker } from "./runtime";

export interface CreatedWorker {
  type: WorkerConfig["type"];
  worker: PetWorker;
}

export async function createWorker(
  name: string,
  config: WorkerConfig,
  onQuit: () => void
): Promise<CreatedWorker> {
  switch (config.type) {
    case "terminal":
      return {
        type: "terminal",
        worker: await createTerminalWorker(config, onQuit)
      };
    case "discord":
      return {
        type: "discord",
        worker: new DiscordWorker(name, config)
      };
  }
}

async function createTerminalWorker(
  config: TerminalWorkerConfig,
  onQuit: () => void
): Promise<TerminalPetWorker> {
  return TerminalPetWorker.create({
    ...(config.color === undefined ? {} : { color: config.color }),
    ...(config.name === undefined ? {} : { name: config.name }),
    ...(config.protocol === undefined ? {} : { protocol: config.protocol }),
    keyboard: true,
    source_label: "pet router",
    on_quit: onQuit
  });
}

class DiscordWorker implements PetWorker {
  readonly #configPath: string;
  readonly #name: string;
  #pet?: DiscordPet;

  constructor(name: string, config: DiscordWorkerConfig) {
    this.#name = name;
    this.#configPath = expandHome(config.config);
  }

  async start(): Promise<void> {
    const config = await loadDiscordConfig(this.#configPath);
    const warning = await permissionsWarning(this.#configPath);
    if (warning) {
      process.stderr.write(`warning: ${warning}\n`);
    }
    const api = new DiscordClient(config.bot_token);
    const username = await api.validateDestination(
      config.guild_id,
      config.channel_id
    );
    this.#pet = await DiscordPet.create(
      api,
      config.channel_id,
      statePath(this.#configPath)
    );
    process.stderr.write(
      `Discord worker ${this.#name} authenticated as @${username}.\n`
    );
  }

  async handle(event: RelayEvent): Promise<void> {
    if (!this.#pet) {
      throw new Error(`Discord worker ${this.#name} is not started`);
    }
    await this.#pet.process(event);
  }

  async stop(): Promise<void> {
    this.#pet = undefined;
  }
}
