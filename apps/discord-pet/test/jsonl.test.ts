import { describe, expect, test } from "bun:test";

import { JsonlDecoder } from "../src/jsonl";
import { parseRelayRecord } from "../src/protocol";

describe("Relay JSONL", () => {
  test("buffers partial records", () => {
    const decoder = new JsonlDecoder(parseRelayRecord);
    const line = JSON.stringify({
      topic: "codex.1",
      session: { session_id: "1" },
      record_id: "jsonl:0",
      operation: "upsert",
      events: [{ type: "message" }]
    });

    expect(decoder.push(new TextEncoder().encode(line.slice(0, 10)))).toEqual([]);
    expect(decoder.push(new TextEncoder().encode(`${line.slice(10)}\n`))).toHaveLength(1);
    expect(decoder.stats.accepted_lines).toBe(1);
  });

  test("counts invalid records without stopping", () => {
    const decoder = new JsonlDecoder(parseRelayRecord);
    decoder.push(new TextEncoder().encode("{}\nnot-json\n"));

    expect(decoder.stats.malformed_lines).toBe(2);
  });

  test("preserves the session path and cwd for terminal input", () => {
    const parsed = parseRelayRecord({
      path: "/tmp/pi/session.jsonl",
      topic: "pi.session-1",
      session: {
        provider: "pi",
        session_id: "session-1",
        cwd: "/tmp/project"
      },
      record_id: "jsonl:0",
      operation: "upsert",
      events: [{ type: "message" }]
    });

    expect(parsed).toMatchObject({
      path: "/tmp/pi/session.jsonl",
      session: {
        provider: "pi",
        session_id: "session-1",
        cwd: "/tmp/project"
      }
    });
  });
});
