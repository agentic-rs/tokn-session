#!/usr/bin/env bun

import { help, parseArgs } from "./cli";
import { expandHome, loadConfig } from "./config";
import { runSupervisor } from "./supervisor";

try {
  const result = parseArgs(process.argv.slice(2));
  if (result.kind === "help") {
    process.stdout.write(`${help()}\n`);
  } else {
    const { config: configPath, ...supervisorOptions } = result.options;
    const config = await loadConfig(expandHome(configPath));
    await runSupervisor(config, supervisorOptions);
  }
} catch (error) {
  process.stderr.write(`error: ${errorMessage(error)}\n`);
  process.exitCode = 1;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
