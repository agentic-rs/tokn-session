import { access } from "node:fs/promises";
import { createInterface } from "node:readline/promises";

import { saveConfig, type DiscordConfig } from "./config";
import { DiscordClient } from "./discord";

interface Prompter {
  text(label: string): Promise<string>;
  secret(label: string): Promise<string>;
  confirm(label: string): Promise<boolean>;
}

interface LoginDependencies {
  prompter?: Prompter;
  config_exists?: (path: string) => Promise<boolean>;
  client_factory?: (token: string) => Pick<DiscordClient, "validateDestination">;
}

export async function login(
  configPath: string,
  dependencies: LoginDependencies = {}
): Promise<void> {
  process.stdout.write(`${setupNote(configPath)}\n`);
  const prompter = dependencies.prompter ?? terminalPrompter;
  const configExists = dependencies.config_exists ?? pathExists;
  if (
    await configExists(configPath)
    && !await prompter.confirm("Replace the existing configuration? [y/N]: ")
  ) {
    process.stdout.write("Login cancelled; existing configuration was not changed.\n");
    return;
  }

  const config: DiscordConfig = {
    bot_token: (await prompter.secret("Bot token (hidden): ")).trim(),
    guild_id: (await prompter.text("Server ID: ")).trim(),
    channel_id: (await prompter.text("Channel ID: ")).trim()
  };
  const client = dependencies.client_factory?.(config.bot_token)
    ?? new DiscordClient(config.bot_token);
  process.stdout.write("\nValidating the bot token and destination with Discord…\n");
  const username = await client.validateDestination(config.guild_id, config.channel_id);
  await saveConfig(configPath, config);
  process.stdout.write(`Authenticated as @${username}.\n`);
  process.stdout.write(`Saved protected configuration to ${configPath}.\n`);
}

export function setupNote(configPath: string): string {
  return `Discord pet login

Before continuing:
  1. Bot token
     Discord Developer Portal → your application → Bot → Reset Token.
  2. Server and channel IDs
     Discord → User Settings → Advanced → enable Developer Mode.
     Right-click the server and target text channel, then choose Copy ID.
  3. Install the bot in that server and grant the target channel:
     View Channel, Send Messages, Create Public Threads,
     Send Messages in Threads, Read Message History, and Embed Links.

No privileged Discord intents are required.
The credentials will be validated before they are saved to:
  ${configPath}`;
}

const terminalPrompter: Prompter = {
  text: readText,
  secret: readSecret,
  async confirm(label: string): Promise<boolean> {
    const answer = await readText(label);
    return ["y", "yes"].includes(answer.trim().toLowerCase());
  }
};

async function readText(label: string): Promise<string> {
  const readline = createInterface({
    input: process.stdin,
    output: process.stdout
  });
  try {
    return await readline.question(label);
  } finally {
    readline.close();
  }
}

async function readSecret(label: string): Promise<string> {
  if (!process.stdin.isTTY || !process.stdin.setRawMode) {
    return readText(label);
  }
  process.stdout.write(label);
  process.stdin.setRawMode(true);
  process.stdin.resume();
  return new Promise<string>((resolvePromise, rejectPromise) => {
    let value = "";
    const finish = (): void => {
      cleanup();
      process.stdout.write("\n");
      resolvePromise(value);
    };
    const cleanup = (): void => {
      process.stdin.off("data", onData);
      process.stdin.setRawMode(false);
      process.stdin.pause();
    };
    const onData = (chunk: Buffer): void => {
      for (const character of chunk.toString("utf8")) {
        if (character === "\u0003") {
          cleanup();
          process.stdout.write("\n");
          rejectPromise(new Error("login cancelled"));
          return;
        }
        if (character === "\r" || character === "\n" || character === "\u0004") {
          finish();
          return;
        }
        if (character === "\u007f" || character === "\b") {
          value = [...value].slice(0, -1).join("");
          continue;
        }
        if (character >= " " && character !== "\u007f") {
          value += character;
        }
      }
    };
    process.stdin.on("data", onData);
  });
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}
