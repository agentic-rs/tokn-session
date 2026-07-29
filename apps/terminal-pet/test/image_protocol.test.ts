import { describe, expect, test } from "bun:test";

import { loadPetArt } from "../src/art";
import {
  PetImageController,
  resolveImageProtocol
} from "../src/image_protocol";

describe("terminal image protocol", () => {
  test("detects safe Kitty terminals and disables images in multiplexers", () => {
    expect(resolveImageProtocol("auto", {
      KITTY_WINDOW_ID: "1"
    })).toBe("kitty");
    expect(resolveImageProtocol("auto", {
      TERM_PROGRAM: "iTerm.app",
      TERM_PROGRAM_VERSION: "3.6.2"
    })).toBe("kitty_file");
    expect(resolveImageProtocol("auto", {
      KITTY_WINDOW_ID: "1",
      TMUX: "/tmp/tmux"
    })).toBe("ansi");
    expect(resolveImageProtocol("kitty", {
      TMUX: "/tmp/tmux"
    })).toBe("ansi");
  });

  test("emits one Kitty transmission per changed pose", async () => {
    const art = await loadPetArt();
    const controller = new PetImageController(art, "kitty");
    const anchor = {
      column: 4,
      row: 3,
      columns: 10,
      rows: 5
    };
    const first = controller.draw("idle", anchor);

    expect(first).toContain("\u001b_Ga=T,t=d,f=100,c=10,r=5");
    expect(controller.draw("idle", anchor)).toBe("");
    expect(controller.draw("blink", anchor)).toContain("\u001b_Ga=d");
    expect(controller.clear()).toContain("\u001b_Ga=d");
  });
});
