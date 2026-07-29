import { describe, expect, test } from "bun:test";

import { parseArgs } from "../src/cli";

describe("CLI", () => {
  test("keeps implicit and explicit run forms", () => {
    expect(parseArgs([]).kind).toBe("run");
    expect(parseArgs(["run"]).kind).toBe("run");
  });

  test("parses login config override", () => {
    expect(parseArgs(["login", "--config", "/tmp/discord.yaml"])).toEqual({
      kind: "login",
      config: "/tmp/discord.yaml"
    });
  });

  test("parses relay options", () => {
    expect(parseArgs([
      "--stdin",
      "--relay-bin",
      "/bin/relay",
      "--codex-dir",
      "/codex",
      "--pi-dir",
      "/pi"
    ])).toMatchObject({
      kind: "run",
      stdin: true,
      relay_bin: "/bin/relay",
      codex_dir: "/codex",
      pi_dir: "/pi"
    });
  });

  test("rejects run-only options for login", () => {
    expect(() => parseArgs(["login", "--stdin"])).toThrow(
      "unknown login option"
    );
  });

  test("routes command help", () => {
    expect(parseArgs(["--help"])).toEqual({ kind: "help", command: "root" });
    expect(parseArgs(["run", "--help"])).toEqual({
      kind: "help",
      command: "run"
    });
    expect(parseArgs(["login", "--help"])).toEqual({
      kind: "help",
      command: "login"
    });
  });
});
