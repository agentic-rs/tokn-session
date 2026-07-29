import type { RelayEvent } from "@tokn/discord-pet/protocol";

export interface PetWorker {
  start(): Promise<void>;
  handle(event: RelayEvent): Promise<void>;
  stop(): Promise<void>;
}

export interface WorkerRuntimeOptions {
  capacity?: number;
  on_error?: (error: unknown) => void;
}

export class WorkerRuntime {
  readonly #capacity: number;
  readonly #onError: (error: unknown) => void;
  readonly #worker: PetWorker;

  #accepting = false;
  #pending = 0;
  #spaceWaiters: Array<() => void> = [];
  #tail = Promise.resolve();

  constructor(worker: PetWorker, options: WorkerRuntimeOptions = {}) {
    this.#worker = worker;
    this.#capacity = options.capacity ?? 256;
    this.#onError = options.on_error ?? (() => {});
    if (!Number.isInteger(this.#capacity) || this.#capacity < 1) {
      throw new Error("worker queue capacity must be a positive integer");
    }
  }

  async start(): Promise<void> {
    await this.#worker.start();
    this.#accepting = true;
  }

  async enqueue(event: RelayEvent): Promise<void> {
    while (this.#accepting && this.#pending >= this.#capacity) {
      await new Promise<void>((resolvePromise) => {
        this.#spaceWaiters.push(resolvePromise);
      });
    }
    if (!this.#accepting) {
      throw new Error("worker is not accepting events");
    }
    this.#pending += 1;
    this.#tail = this.#tail
      .then(() => this.#worker.handle(event))
      .catch((error: unknown) => this.#onError(error))
      .finally(() => {
        this.#pending -= 1;
        this.#spaceWaiters.shift()?.();
      });
  }

  async stop(): Promise<void> {
    this.#accepting = false;
    for (const wake of this.#spaceWaiters.splice(0)) {
      wake();
    }
    await this.#tail;
    await this.#worker.stop();
  }
}
