import { describe, expect, test } from "bun:test";

import { consumeJsonl, JsonlDecoder } from "../src/jsonl";
import { parseRelayRecord } from "../src/protocol";

const encoder = new TextEncoder();
const event = JSON.stringify({
  topic: "codex.session-1",
  session: {
    session_id: "session-1"
  },
  record_id: "jsonl:0",
  operation: "upsert",
  events: [{ type: "reasoning" }]
});

describe("JsonlDecoder", () => {
  test("decodes records split across arbitrary chunks", () => {
    const decoder = new JsonlDecoder(parseRelayRecord);
    expect(decoder.push(encoder.encode(event.slice(0, 17)))).toEqual([]);
    const values = decoder.push(encoder.encode(`${event.slice(17)}\r\n`));

    expect(values).toHaveLength(1);
    expect(values[0]?.topic).toBe("codex.session-1");
    expect(decoder.stats).toEqual({
      received_lines: 1,
      accepted_lines: 1,
      malformed_lines: 0,
      oversized_lines: 0
    });
  });

  test("accepts a final record without a newline", () => {
    const decoder = new JsonlDecoder(parseRelayRecord);
    expect(decoder.push(encoder.encode(event))).toEqual([]);
    expect(decoder.finish()).toHaveLength(1);
  });

  test("preserves distinct project, folder, and repository names", () => {
    const decoder = new JsonlDecoder(parseRelayRecord);
    const enriched = JSON.stringify({
      topic: "codex.session-1",
      session: {
        session_id: "session-1",
        project: {
          name: "tokn",
          project_name: "llm-router_2",
          folder: "/worktrees/59e1/llm-router",
          folder_name: "llm-router",
          repository_name: "tokn"
        }
      },
      record_id: "jsonl:0",
      operation: "upsert",
      events: [{ type: "reasoning" }]
    });

    const values = decoder.push(encoder.encode(`${enriched}\n`));

    expect(values[0]?.session.project).toEqual({
      name: "tokn",
      project_name: "llm-router_2",
      folder: "/worktrees/59e1/llm-router",
      folder_name: "llm-router",
      repository_name: "tokn"
    });
  });

  test("counts invalid JSON and invalid RelayRecord shapes", () => {
    const decoder = new JsonlDecoder(parseRelayRecord);
    const values = decoder.push(encoder.encode("{bad}\n{}\n\n"));

    expect(values).toEqual([]);
    expect(decoder.stats.malformed_lines).toBe(2);
    expect(decoder.stats.received_lines).toBe(2);
  });

  test("discards an oversized unterminated line", () => {
    const maxLineLength = event.length + 5;
    const decoder = new JsonlDecoder(parseRelayRecord, maxLineLength);
    expect(decoder.push(encoder.encode("x".repeat(maxLineLength + 1)))).toEqual([]);
    expect(decoder.push(encoder.encode("still oversized"))).toEqual([]);
    expect(decoder.push(encoder.encode(`\n${event}\n`))).toHaveLength(1);
    expect(decoder.stats.oversized_lines).toBe(1);
  });

  test("discards a complete oversized line before parsing", () => {
    const decoder = new JsonlDecoder(parseRelayRecord, 8);
    expect(decoder.push(encoder.encode(`${event}\n`))).toEqual([]);
    expect(decoder.stats).toEqual({
      received_lines: 1,
      accepted_lines: 0,
      malformed_lines: 0,
      oversized_lines: 1
    });
  });

  test("cancels an open stream when aborted", async () => {
    let cancelled = false;
    const stream = new ReadableStream<Uint8Array>({
      cancel() {
        cancelled = true;
      }
    });
    const abort = new AbortController();
    const consumption = consumeJsonl(
      stream,
      new JsonlDecoder(parseRelayRecord),
      () => {},
      abort.signal
    );

    abort.abort();
    await consumption;
    expect(cancelled).toBe(true);
  });
});
