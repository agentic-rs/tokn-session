import {
  PetImageController,
  resolveImageProtocol,
  type ImageProtocolOption
} from "./image_protocol";
import { TerminalKeyDecoder, type PetKeyAction } from "./keys";
import { loadPetArt, selectPose, type PetArt } from "./art";
import { focusSnapshot, moveFocusTopic, type FocusDirection } from "./navigation";
import type { RelayEvent } from "./protocol";
import { renderScreen, type RenderMeta } from "./renderer";
import { PetStore, type PetSnapshot } from "./state";
import { TerminalSurface } from "./terminal";

const KEY_SEQUENCE_TIMEOUT_MS = 50;

export interface TerminalPetWorkerOptions {
  color?: boolean;
  keyboard?: boolean;
  name?: string;
  protocol?: ImageProtocolOption;
  source_label?: string;
  on_quit?: () => void;
}

export class TerminalPetWorker {
  readonly #art: PetArt;
  readonly #color: boolean;
  readonly #imageController: PetImageController;
  readonly #imageProtocol;
  readonly #keyboard: boolean;
  readonly #keyDecoder = new TerminalKeyDecoder();
  readonly #meta: RenderMeta;
  readonly #name: string;
  readonly #onQuit: () => void;
  readonly #store = new PetStore();
  readonly #surface = new TerminalSurface();

  #frameTimer?: ReturnType<typeof setInterval>;
  #keyFlushTimer?: ReturnType<typeof setTimeout>;
  #rawMode = false;
  #selectedTopic?: string;
  #started = false;

  private constructor(art: PetArt, options: TerminalPetWorkerOptions) {
    this.#art = art;
    this.#color = options.color
      ?? (!process.env.NO_COLOR && process.env.TERM !== "dumb");
    this.#keyboard = options.keyboard ?? true;
    this.#name = options.name ?? "Hachiware";
    this.#imageProtocol = resolveImageProtocol(options.protocol ?? "auto");
    this.#imageController = new PetImageController(art, this.#imageProtocol);
    this.#onQuit = options.on_quit ?? (() => {});
    this.#meta = {
      source_label: options.source_label ?? "pet router",
      control_mode: this.#keyboard && process.stdin.isTTY
        ? "relay"
        : "signal_only"
    };
  }

  static async create(
    options: TerminalPetWorkerOptions = {}
  ): Promise<TerminalPetWorker> {
    return new TerminalPetWorker(await loadPetArt(), options);
  }

  async start(): Promise<void> {
    if (this.#started) {
      return;
    }
    this.#started = true;
    this.#surface.enter();
    this.#render();
    this.#frameTimer = setInterval(() => this.#render(), 120);
    process.stdout.on("resize", this.#onResize);

    if (this.#keyboard && process.stdin.isTTY) {
      process.stdin.setRawMode(true);
      process.stdin.resume();
      process.stdin.on("data", this.#onKey);
      this.#rawMode = true;
    }
  }

  async handle(event: RelayEvent): Promise<void> {
    if (!this.#started) {
      throw new Error("terminal pet worker is not started");
    }
    this.#store.ingest(event);
    this.#render();
  }

  async stop(): Promise<void> {
    if (!this.#started) {
      return;
    }
    this.#started = false;
    if (this.#frameTimer) {
      clearInterval(this.#frameTimer);
      this.#frameTimer = undefined;
    }
    if (this.#keyFlushTimer) {
      clearTimeout(this.#keyFlushTimer);
      this.#keyFlushTimer = undefined;
    }
    process.stdout.off("resize", this.#onResize);
    process.stdin.off("data", this.#onKey);
    if (this.#rawMode) {
      process.stdin.setRawMode(false);
      process.stdin.pause();
      this.#rawMode = false;
    }
    process.stdout.write(this.#imageController.clear());
    this.#surface.leave();
  }

  readonly #onResize = (): void => {
    if (!this.#started) {
      return;
    }
    process.stdout.write(this.#imageController.clear());
    this.#surface.invalidate();
    this.#render();
  };

  readonly #onKey = (chunk: Buffer): void => {
    if (!this.#started) {
      return;
    }
    if (this.#keyFlushTimer) {
      clearTimeout(this.#keyFlushTimer);
      this.#keyFlushTimer = undefined;
    }
    this.#dispatchActions(this.#keyDecoder.push(chunk));
    if (this.#started && this.#keyDecoder.has_pending_sequence) {
      this.#keyFlushTimer = setTimeout(() => {
        this.#keyFlushTimer = undefined;
        if (this.#started) {
          this.#dispatchActions(this.#keyDecoder.flush());
        }
      }, KEY_SEQUENCE_TIMEOUT_MS);
    }
  };

  #snapshot(nowMs: number): PetSnapshot {
    const snapshot = this.#store.snapshot(nowMs);
    if (
      this.#selectedTopic
      && !snapshot.sessions.some(
        (session) => session.topic === this.#selectedTopic
      )
    ) {
      this.#selectedTopic = undefined;
    }
    return focusSnapshot(snapshot, this.#selectedTopic);
  }

  #render(): void {
    if (!this.#started) {
      return;
    }
    const nowMs = Date.now();
    const snapshot = this.#snapshot(nowMs);
    const pose = selectPose(snapshot.state, snapshot.state_changed_at, nowMs);
    const screen = renderScreen(snapshot, this.#art[pose].ansi, {
      ...this.#meta,
      focus_mode: this.#selectedTopic ? "manual" : "auto"
    }, {
      columns: process.stdout.columns ?? 80,
      rows: process.stdout.rows ?? 24,
      color: this.#color,
      image_protocol: this.#imageProtocol,
      name: this.#name,
      now_ms: nowMs
    });
    this.#surface.render(screen.lines);
    process.stdout.write(
      screen.image_anchor
        ? this.#imageController.draw(pose, screen.image_anchor)
        : this.#imageController.clear()
    );
  }

  #moveFocus(direction: FocusDirection): void {
    this.#selectedTopic = moveFocusTopic(
      this.#store.snapshot(Date.now()),
      this.#selectedTopic,
      direction
    );
    this.#render();
  }

  #dispatchActions(actions: PetKeyAction[]): void {
    for (const action of actions) {
      switch (action) {
        case "quit":
          this.#onQuit();
          return;
        case "acknowledge": {
          const topic = this.#snapshot(Date.now()).focus?.topic;
          if (topic) {
            this.#store.acknowledge(topic);
            this.#render();
          }
          break;
        }
        case "select_next":
          this.#moveFocus("next");
          break;
        case "select_previous":
          this.#moveFocus("previous");
          break;
        case "auto_focus":
          this.#selectedTopic = undefined;
          this.#render();
          break;
      }
    }
  }
}
