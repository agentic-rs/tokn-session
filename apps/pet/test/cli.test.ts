import { describe, expect, test } from "bun:test";

import { parseArgs } from "../src/cli";

describe("pet CLI", () => {
  test("parses routing and Relay options", () => {
    expect(parseArgs([
      "--config",
      "/tmp/pet.yaml",
      "--stdin",
      "--queue-capacity",
      "32"
    ])).toEqual({
      kind: "run",
      options: {
        config: "/tmp/pet.yaml",
        stdin: true,
        queue_capacity: 32
      }
    });
  });

  test("rejects invalid queue capacity", () => {
    expect(() => parseArgs(["--queue-capacity", "0"])).toThrow(
      "positive integer"
    );
  });
});
