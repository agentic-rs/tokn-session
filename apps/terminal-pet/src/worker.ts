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
import {
  PiInputBroker,
  TerminalInputEditor,
  type TerminalInputEvent
} from "./input";

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
  readonly #inputEditor = new TerminalInputEditor();
  readonly #piInput = new PiInputBroker();
  readonly #store = new PetStore();
  readonly #surface = new TerminalSurface();

  #frameTimer?: ReturnType<typeof setInterval>;
  #keyFlushTimer?: ReturnType<typeof setTimeout>;
  #rawMode = false;
  #inputTopic?: string;
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
    this.#piInput.observe(event);
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
    this.#inputEditor.cancel();
    this.#inputTopic = undefined;
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
    for (let index = 0; index < chunk.length && this.#started; index += 1) {
      if (this.#inputEditor.active) {
        this.#handleInputEvents(this.#inputEditor.feed(chunk.subarray(index)));
        break;
      }
      this.#dispatchActions(this.#keyDecoder.push(chunk.subarray(index, index + 1)));
    }
    if (this.#started && !this.#inputEditor.active && this.#keyDecoder.has_pending_sequence) {
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
      focus_mode: this.#selectedTopic ? "manual" : "auto",
      input_active: this.#inputEditor.active,
      input_line: this.#inputEditor.active ? this.#inputEditor.value : undefined
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
    if (this.#inputEditor.active) {
      return;
    }
    this.#selectedTopic = moveFocusTopic(
      this.#store.snapshot(Date.now()),
      this.#selectedTopic,
      direction
    );
    this.#render();
  }

  #beginInput(): void {
    const focus = this.#snapshot(Date.now()).focus;
    if (!focus) {
      this.#meta.input_status = "no session is selected";
      this.#render();
      return;
    }
    const provider = focus.provider ?? focus.topic.split(".", 1)[0];
    if (provider?.toLowerCase() !== "pi") {
      this.#meta.input_status = "terminal input currently supports Pi sessions only";
      this.#render();
      return;
    }
    this.#inputTopic = focus.topic;
    this.#meta.input_status = undefined;
    this.#meta.diagnostic = undefined;
    this.#inputEditor.begin();
    this.#render();
  }

  #handleInputEvents(events: TerminalInputEvent[]): void {
    for (const event of events) {
      switch (event.type) {
        case "changed":
          this.#render();
          break;
        case "cancelled":
          this.#inputTopic = undefined;
          this.#meta.input_status = undefined;
          this.#render();
          break;
        case "submitted": {
          const topic = this.#inputTopic;
          this.#inputTopic = undefined;
          if (event.text.trim().length === 0) {
            this.#meta.input_status = "message cannot be empty";
            this.#render();
            break;
          }
          this.#meta.input_status = "sending input to Pi…";
          this.#render();
          if (!topic) {
            this.#meta.input_status = "no session is selected";
            this.#render();
            break;
          }
          void this.#piInput.submit(topic, event.text).then(
            () => {
              if (this.#started) {
                this.#meta.input_status = "Pi input sent";
                this.#render();
              }
            },
            (error: unknown) => {
              if (this.#started) {
                this.#meta.input_status = undefined;
                this.#meta.diagnostic = error instanceof Error
                  ? error.message
                  : String(error);
                this.#render();
              }
            }
          );
          break;
        }
      }
    }
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
        case "begin_input":
          this.#beginInput();
          break;
      }
    }
  }
}
