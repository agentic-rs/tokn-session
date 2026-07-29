import { describe, expect, test } from "bun:test";

import { ART_POSES, loadPetArt, selectPose } from "../src/art";

describe("pet art", () => {
  test("loads every generated frame and builds an ANSI fallback", async () => {
    const art = await loadPetArt();
    for (const pose of ART_POSES) {
      expect(art[pose].png_base64.length).toBeGreaterThan(100);
      expect(art[pose].ansi).toHaveLength(14);
      expect(art[pose].ansi.some((row) => row.some(Boolean))).toBe(true);
    }
  });

  test("uses the official-style idle blink cadence", () => {
    expect(selectPose("idle", 0, 0)).toBe("idle");
    expect(selectPose("idle", 0, 1_679)).toBe("idle");
    expect(selectPose("idle", 0, 1_680)).toBe("blink");
  });

  test("maps semantic states to art poses then settles to idle", () => {
    expect(selectPose("running", 0, 0)).toBe("running");
    expect(selectPose("needs_input", 0, 0)).toBe("waiting");
    expect(selectPose("ready", 0, 0)).toBe("ready");
    expect(selectPose("blocked", 0, 0)).toBe("blocked");
    expect(["idle", "blink"]).toContain(selectPose("running", 0, 3_000));
  });
});
