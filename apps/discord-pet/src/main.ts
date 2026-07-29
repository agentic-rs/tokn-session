#!/usr/bin/env bun

import { help, parseArgs, type RunCommand } from "./cli";
import {
  loadConfig,
  permissionsWarning,
  statePath
} from "./config";
import { DiscordClient } from "./discord";
import { login } from "./login";
import { DiscordPet } from "./pet";
import { followRelay } from "./relay";

try {
  const command = parseArgs(process.argv.slice(2));
  switch (command.kind) {
    case "help":
      process.stdout.write(`${help(command.command)}\n`);
      break;
    case "login":
      await login(command.config);
      break;
    case "run":
      await run(command);
      break;
  }
} catch (error) {
  process.stderr.write(`error: ${errorMessage(error)}\n`);
  process.exitCode = 1;
}

async function run(command: RunCommand): Promise<void> {
  const config = await loadConfig(command.config);
  const warning = await permissionsWarning(command.config);
  if (warning) {
    process.stderr.write(`warning: ${warning}\n`);
  }
  const api = new DiscordClient(config.bot_token);
  const username = await api.validateDestination(config.guild_id, config.channel_id);
  const pet = await DiscordPet.create(
    api,
    config.channel_id,
    statePath(command.config)
  );
  process.stderr.write(`Discord pet authenticated as @${username}; following Relay events.\n`);
  await followRelay(command, (event) => pet.process(event));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
