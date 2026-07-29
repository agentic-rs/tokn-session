import type { RelayEvent } from "@tokn/discord-pet/protocol";
import { followRelay, type RelayOptions } from "@tokn/discord-pet/relay";

import type { PetConfig } from "./config";
import { RuleEngine } from "./rules";
import { WorkerRuntime } from "./runtime";
import { createWorker } from "./workers";

export interface SupervisorOptions extends Omit<RelayOptions, "signal"> {
  queue_capacity?: number;
}

interface NamedRuntime {
  name: string;
  type: "terminal" | "discord";
  runtime: WorkerRuntime;
}

export async function runSupervisor(
  config: PetConfig,
  options: SupervisorOptions
): Promise<void> {
  const abort = new AbortController();
  const rules = new RuleEngine(config.rules);
  const workers = new Map<string, NamedRuntime>();
  for (const [name, workerConfig] of Object.entries(config.workers)) {
    const created = await createWorker(name, workerConfig, () => abort.abort());
    workers.set(name, {
      name,
      type: created.type,
      runtime: new WorkerRuntime(created.worker, {
        ...(options.queue_capacity === undefined
          ? {}
          : { capacity: options.queue_capacity }),
        on_error: (error) => {
          process.stderr.write(
            `worker ${name} failed to handle an event: ${errorMessage(error)}\n`
          );
        }
      })
    });
  }

  const ordered = [...workers.values()].sort((left, right) => {
    return Number(left.type === "terminal") - Number(right.type === "terminal");
  });
  const started: NamedRuntime[] = [];
  try {
    for (const worker of ordered) {
      await worker.runtime.start();
      started.push(worker);
    }
    await followRelay({
      stdin: options.stdin,
      ...(options.relay_bin === undefined
        ? {}
        : { relay_bin: options.relay_bin }),
      ...(options.codex_dir === undefined
        ? {}
        : { codex_dir: options.codex_dir }),
      ...(options.pi_dir === undefined
        ? {}
        : { pi_dir: options.pi_dir }),
      diagnostics: ordered.some((worker) => worker.type === "terminal")
        ? "discard"
        : "inherit",
      signal: abort.signal
    }, async (event) => {
      await routeEvent(event, rules, workers);
    });
  } finally {
    abort.abort();
    for (const worker of started.reverse()) {
      await worker.runtime.stop();
    }
  }
}

async function routeEvent(
  event: RelayEvent,
  rules: RuleEngine,
  workers: Map<string, NamedRuntime>
): Promise<void> {
  await Promise.all(rules.targets(event).map(async (name) => {
    const worker = workers.get(name);
    if (!worker) {
      throw new Error(`rule selected unknown worker ${name}`);
    }
    await worker.runtime.enqueue(event);
  }));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
