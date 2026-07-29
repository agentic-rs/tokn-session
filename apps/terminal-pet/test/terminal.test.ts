import { describe, expect, test } from "bun:test";

import { TerminalSurface } from "../src/terminal";

describe("TerminalSurface", () => {
  test("rewrites only changed rows and restores terminal state", () => {
    const writes: string[] = [];
    const surface = new TerminalSurface({
      write(value) {
        writes.push(value);
        return true;
      }
    });
    surface.enter();
    surface.render(["one", "two"]);
    const afterFirstRender = writes.length;
    surface.render(["one", "two"]);
    expect(writes).toHaveLength(afterFirstRender);

    surface.render(["one", "changed"]);
    expect(writes.at(-1)).toContain("\u001b[2;1H");
    expect(writes.at(-1)).not.toContain("\u001b[1;1H");

    surface.leave();
    expect(writes.at(-1)).toContain("\u001b[?25h");
    expect(writes.at(-1)).toContain("\u001b[?1049l");
  });
});
