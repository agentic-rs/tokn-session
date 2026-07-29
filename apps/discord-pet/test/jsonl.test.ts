import { describe, expect, test } from "bun:test";

import { JsonlDecoder } from "../src/jsonl";
import { parseRelayEvent } from "../src/protocol";

describe("Relay JSONL", () => {
  test("buffers partial records", () => {
    const decoder = new JsonlDecoder(parseRelayEvent);
    const line = JSON.stringify({
      topic: "codex.1",
      session: { session_id: "1" },
      event: { type: "message" }
    });

    expect(decoder.push(new TextEncoder().encode(line.slice(0, 10)))).toEqual([]);
    expect(decoder.push(new TextEncoder().encode(`${line.slice(10)}\n`))).toHaveLength(1);
    expect(decoder.stats.accepted_lines).toBe(1);
  });

  test("counts invalid records without stopping", () => {
    const decoder = new JsonlDecoder(parseRelayEvent);
    decoder.push(new TextEncoder().encode("{}\nnot-json\n"));

    expect(decoder.stats.malformed_lines).toBe(2);
  });
});
