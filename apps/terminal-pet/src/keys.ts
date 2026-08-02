export const PET_KEY_ACTIONS = [
  "quit",
  "acknowledge",
  "select_next",
  "select_previous",
  "auto_focus",
  "begin_input"
] as const;

export type PetKeyAction = typeof PET_KEY_ACTIONS[number];

type DecoderState =
  | "ground"
  | "escape"
  | "escape_intermediate"
  | "csi"
  | "ss3"
  | "control_string"
  | "control_string_escape";

const ESCAPE = 0x1b;

export class TerminalKeyDecoder {
  #state: DecoderState = "ground";
  #controlStringAllowsBel = false;

  get has_pending_sequence(): boolean {
    return this.#state !== "ground";
  }

  push(chunk: Uint8Array): PetKeyAction[] {
    const actions: PetKeyAction[] = [];
    for (const byte of chunk) {
      this.#consume(byte, actions);
    }
    return actions;
  }

  flush(): PetKeyAction[] {
    const action = this.#state === "escape" ? "quit" : undefined;
    this.#state = "ground";
    this.#controlStringAllowsBel = false;
    return action ? [action] : [];
  }

  #consume(byte: number, actions: PetKeyAction[]): void {
    switch (this.#state) {
      case "ground":
        this.#consumeGround(byte, actions);
        return;
      case "escape":
        this.#consumeEscape(byte, actions);
        return;
      case "escape_intermediate":
        this.#consumeEscapeIntermediate(byte);
        return;
      case "csi":
      case "ss3":
        this.#consumeControlSequence(byte, actions);
        return;
      case "control_string":
        this.#consumeControlString(byte);
        return;
      case "control_string_escape":
        this.#consumeControlStringEscape(byte);
        return;
    }
  }

  #consumeGround(byte: number, actions: PetKeyAction[]): void {
    if (byte === ESCAPE) {
      this.#state = "escape";
      return;
    }
    const action = actionForByte(byte);
    if (action) {
      actions.push(action);
    }
  }

  #consumeEscape(byte: number, actions: PetKeyAction[]): void {
    if (byte === ESCAPE) {
      actions.push("quit");
      return;
    }
    if (byte === 0x5b) {
      this.#state = "csi";
      return;
    }
    if (byte === 0x4f) {
      this.#state = "ss3";
      return;
    }
    if (isControlStringIntroducer(byte)) {
      this.#beginControlString(byte === 0x5d);
      return;
    }
    if (isEscapeIntermediate(byte)) {
      this.#state = "escape_intermediate";
      return;
    }

    this.#state = "ground";
  }

  #consumeEscapeIntermediate(byte: number): void {
    if (byte === ESCAPE) {
      this.#state = "escape";
    } else if (isEscapeSequenceFinal(byte)) {
      this.#state = "ground";
    } else if (!isEscapeIntermediate(byte)) {
      this.#state = "ground";
    }
  }

  #consumeControlSequence(byte: number, actions: PetKeyAction[]): void {
    if (byte === ESCAPE) {
      this.#state = "escape";
      return;
    }
    if (!isControlSequenceFinal(byte)) {
      return;
    }

    if (byte === 0x41) {
      actions.push("select_previous");
    } else if (byte === 0x42) {
      actions.push("select_next");
    }
    this.#state = "ground";
  }

  #consumeControlString(byte: number): void {
    if (
      this.#controlStringAllowsBel && byte === 0x07
    ) {
      this.#state = "ground";
      this.#controlStringAllowsBel = false;
    } else if (byte === ESCAPE) {
      this.#state = "control_string_escape";
    }
  }

  #consumeControlStringEscape(byte: number): void {
    if (byte === 0x5c) {
      this.#state = "ground";
      this.#controlStringAllowsBel = false;
    } else if (byte !== ESCAPE) {
      this.#state = "control_string";
    }
  }

  #beginControlString(allowsBel: boolean): void {
    this.#state = "control_string";
    this.#controlStringAllowsBel = allowsBel;
  }
}

function actionForByte(byte: number): PetKeyAction | undefined {
  switch (byte) {
    case 0x03:
    case 0x71:
      return "quit";
    case 0x63:
      return "acknowledge";
    case 0x6a:
      return "select_next";
    case 0x6b:
      return "select_previous";
    case 0x61:
      return "auto_focus";
    case 0x0a:
    case 0x0d:
      return "begin_input";
    default:
      return undefined;
  }
}

function isControlStringIntroducer(byte: number): boolean {
  return byte === 0x50
    || byte === 0x58
    || byte === 0x5d
    || byte === 0x5e
    || byte === 0x5f;
}

function isControlSequenceFinal(byte: number): boolean {
  return byte >= 0x40 && byte <= 0x7e;
}

function isEscapeIntermediate(byte: number): boolean {
  return byte >= 0x20 && byte <= 0x2f;
}

function isEscapeSequenceFinal(byte: number): boolean {
  return byte >= 0x30 && byte <= 0x7e;
}
