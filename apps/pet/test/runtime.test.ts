import { describe, expect, test } from "bun:test";

import {
  WorkerRuntime,
  type PetWorker
} from "../src/runtime";
import { relayEvent } from "./fixtures";

describe("WorkerRuntime", () => {
  test("serializes a worker lifecycle and event handling", async () => {
    const calls: string[] = [];
    const worker: PetWorker = {
      async start() {
        calls.push("start");
      },
      async handle(event) {
        await Bun.sleep(1);
        calls.push(event.topic);
      },
      async stop() {
        calls.push("stop");
      }
    };
    const runtime = new WorkerRuntime(worker);

    await runtime.start();
    await runtime.enqueue(relayEvent({ topic: "first" }));
    await runtime.enqueue(relayEvent({ topic: "second" }));
    await runtime.stop();

    expect(calls).toEqual(["start", "first", "second", "stop"]);
  });

  test("isolates handler failures and continues the queue", async () => {
    const handled: string[] = [];
    const errors: string[] = [];
    const runtime = new WorkerRuntime({
      async start() {},
      async handle(event) {
        if (event.topic === "bad") {
          throw new Error("boom");
        }
        handled.push(event.topic);
      },
      async stop() {}
    }, {
      on_error: (error) => {
        errors.push(error instanceof Error ? error.message : String(error));
      }
    });

    await runtime.start();
    await runtime.enqueue(relayEvent({ topic: "bad" }));
    await runtime.enqueue(relayEvent({ topic: "good" }));
    await runtime.stop();

    expect(errors).toEqual(["boom"]);
    expect(handled).toEqual(["good"]);
  });

  test("applies backpressure when the bounded queue is full", async () => {
    let releaseFirst!: () => void;
    let markStarted!: () => void;
    const firstStarted = new Promise<void>((resolvePromise) => {
      markStarted = resolvePromise;
    });
    const firstReleased = new Promise<void>((resolvePromise) => {
      releaseFirst = resolvePromise;
    });
    const handled: string[] = [];
    const runtime = new WorkerRuntime(
      {
        async start() {},
        async handle(event) {
          if (event.topic === "first") {
            markStarted();
            await firstReleased;
          }
          handled.push(event.topic);
        },
        async stop() {}
      },
      { capacity: 1 }
    );

    await runtime.start();
    await runtime.enqueue(relayEvent({ topic: "first" }));
    await firstStarted;

    let secondEnqueued = false;
    const second = runtime.enqueue(relayEvent({ topic: "second" })).then(() => {
      secondEnqueued = true;
    });
    await Bun.sleep(1);
    expect(secondEnqueued).toBeFalse();

    releaseFirst();
    await second;
    await runtime.stop();

    expect(handled).toEqual(["first", "second"]);
  });
});
