import { describe, expect, test } from "bun:test";

import { TerminalKeyDecoder } from "../src/keys";

const encoder = new TextEncoder();

describe("TerminalKeyDecoder", () => {
  test("maps direct keys to pet actions", () => {
    const decoder = new TerminalKeyDecoder();

    expect(decoder.push(Uint8Array.of(
      0x71,
      0x03,
      0x63,
      0x6a,
      0x6b,
      0x61,
      0x0d
    ))).toEqual([
      "quit",
      "quit",
      "acknowledge",
      "select_next",
      "select_previous",
      "auto_focus",
      "begin_input"
    ]);
    expect(decoder.has_pending_sequence).toBe(false);
  });

  test("emits standalone Escape only when flushed", () => {
    const decoder = new TerminalKeyDecoder();

    expect(decoder.push(Uint8Array.of(0x1b))).toEqual([]);
    expect(decoder.has_pending_sequence).toBe(true);
    expect(decoder.flush()).toEqual(["quit"]);
    expect(decoder.has_pending_sequence).toBe(false);
    expect(decoder.flush()).toEqual([]);
  });

  test("decodes CSI arrows split across chunks", () => {
    const decoder = new TerminalKeyDecoder();

    expect(decoder.push(Uint8Array.of(0x1b))).toEqual([]);
    expect(decoder.push(encoder.encode("["))).toEqual([]);
    expect(decoder.has_pending_sequence).toBe(true);
    expect(decoder.push(encoder.encode("A"))).toEqual(["select_previous"]);
    expect(decoder.has_pending_sequence).toBe(false);

    expect(decoder.push(encoder.encode("\u001b[1;5"))).toEqual([]);
    expect(decoder.has_pending_sequence).toBe(true);
    expect(decoder.push(encoder.encode("B"))).toEqual(["select_next"]);
    expect(decoder.has_pending_sequence).toBe(false);
  });

  test("decodes SS3 arrows split across chunks", () => {
    const decoder = new TerminalKeyDecoder();

    expect(decoder.push(encoder.encode("\u001bO"))).toEqual([]);
    expect(decoder.has_pending_sequence).toBe(true);
    expect(decoder.push(encoder.encode("A"))).toEqual(["select_previous"]);

    expect(decoder.push(encoder.encode("\u001b"))).toEqual([]);
    expect(decoder.push(encoder.encode("O"))).toEqual([]);
    expect(decoder.push(encoder.encode("B"))).toEqual(["select_next"]);
    expect(decoder.has_pending_sequence).toBe(false);
  });

  test("ignores other escape sequences without consuming following keys", () => {
    const decoder = new TerminalKeyDecoder();

    expect(decoder.push(encoder.encode("\u001b[Cq"))).toEqual(["quit"]);
    expect(decoder.push(encoder.encode("\u001bOPc"))).toEqual(["acknowledge"]);

    expect(decoder.push(encoder.encode("\u001b("))).toEqual([]);
    expect(decoder.has_pending_sequence).toBe(true);
    expect(decoder.push(encoder.encode("cj"))).toEqual(["select_next"]);
    expect(decoder.has_pending_sequence).toBe(false);
  });

  test("ignores control strings and their embedded key bytes", () => {
    const decoder = new TerminalKeyDecoder();

    expect(decoder.push(encoder.encode("\u001b]0;qca"))).toEqual([]);
    expect(decoder.has_pending_sequence).toBe(true);
    expect(decoder.push(Uint8Array.of(0x07, 0x6a))).toEqual(["select_next"]);

    expect(decoder.push(encoder.encode("\u001bPqca"))).toEqual([]);
    expect(decoder.push(Uint8Array.of(0x1b))).toEqual([]);
    expect(decoder.has_pending_sequence).toBe(true);
    expect(decoder.push(encoder.encode("\\k"))).toEqual(["select_previous"]);
    expect(decoder.has_pending_sequence).toBe(false);

    expect(decoder.push(encoder.encode("\u001b]Üq\u001b\\j"))).toEqual([
      "select_next"
    ]);
  });

  test("does not mistake UTF-8 continuation bytes for terminal controls", () => {
    const decoder = new TerminalKeyDecoder();

    expect(decoder.push(encoder.encode("ÛA"))).toEqual([]);
    expect(decoder.has_pending_sequence).toBe(false);
    expect(decoder.push(encoder.encode("🙂q"))).toEqual(["quit"]);
    expect(decoder.has_pending_sequence).toBe(false);
  });

  test("only lets OSC use BEL as a string terminator", () => {
    const decoder = new TerminalKeyDecoder();

    expect(decoder.push(encoder.encode("\u001b]title\u0007q"))).toEqual(["quit"]);
    expect(decoder.push(encoder.encode("\u001bP\u0007q"))).toEqual([]);
    expect(decoder.has_pending_sequence).toBe(true);
    expect(decoder.push(encoder.encode("\u001b\\k"))).toEqual(["select_previous"]);
  });

  test("flushes incomplete non-Escape sequences without an action", () => {
    const decoder = new TerminalKeyDecoder();

    expect(decoder.push(encoder.encode("\u001b["))).toEqual([]);
    expect(decoder.has_pending_sequence).toBe(true);
    expect(decoder.flush()).toEqual([]);
    expect(decoder.has_pending_sequence).toBe(false);

    expect(decoder.push(encoder.encode("a"))).toEqual(["auto_focus"]);
  });
});
