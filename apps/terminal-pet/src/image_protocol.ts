import type { ArtPose, PetArt } from "./art";

const ESC = "\u001b";
const STRING_TERMINATOR = `${ESC}\\`;
const IMAGE_ID = 19_860_729;
const KITTY_CHUNK_SIZE = 4_096;

export const IMAGE_PROTOCOLS = [
  "auto",
  "ansi",
  "kitty",
  "kitty_file"
] as const;

export type ImageProtocolOption = typeof IMAGE_PROTOCOLS[number];
export type ImageProtocol = Exclude<ImageProtocolOption, "auto">;

export interface ImageAnchor {
  column: number;
  row: number;
  columns: number;
  rows: number;
}

export function resolveImageProtocol(
  selection: ImageProtocolOption,
  environment: NodeJS.ProcessEnv = process.env
): ImageProtocol {
  if (
    environment.TMUX
    || environment.TMUX_PANE
    || environment.ZELLIJ
    || environment.ZELLIJ_SESSION_NAME
  ) {
    return "ansi";
  }
  if (selection !== "auto") {
    return selection;
  }

  const termProgram = environment.TERM_PROGRAM?.toLowerCase() ?? "";
  if (termProgram.includes("iterm") && versionAtLeast(environment.TERM_PROGRAM_VERSION, [3, 6, 0])) {
    return "kitty_file";
  }
  const terminalHint = [
    environment.TERM,
    environment.TERM_PROGRAM,
    environment.LC_TERMINAL
  ].filter(Boolean).join(" ").toLowerCase();
  if (
    environment.KITTY_WINDOW_ID
    || environment.WEZTERM_EXECUTABLE
    || environment.WEZTERM_VERSION
    || terminalHint.includes("kitty")
    || terminalHint.includes("ghostty")
    || terminalHint.includes("wezterm")
  ) {
    return "kitty";
  }
  return "ansi";
}

export class PetImageController {
  #lastDraw?: string;

  constructor(
    private readonly art: PetArt,
    private readonly protocol: ImageProtocol
  ) {}

  draw(pose: ArtPose, anchor: ImageAnchor): string {
    if (this.protocol === "ansi") {
      return "";
    }
    const drawKey = [
      pose,
      anchor.column,
      anchor.row,
      anchor.columns,
      anchor.rows
    ].join(":");
    if (drawKey === this.#lastDraw) {
      return "";
    }
    this.#lastDraw = drawKey;

    const frame = this.art[pose];
    const payload = this.protocol === "kitty_file"
      ? kittyFile(frame.path_base64, anchor.columns, anchor.rows)
      : kittyData(frame.png_base64, anchor.columns, anchor.rows);
    return [
      saveCursor(),
      kittyDelete(),
      cursorTo(anchor.row, anchor.column),
      payload,
      restoreCursor()
    ].join("");
  }

  clear(): string {
    if (this.protocol === "ansi" || this.#lastDraw === undefined) {
      return "";
    }
    this.#lastDraw = undefined;
    return kittyDelete();
  }

  invalidate(): void {
    this.#lastDraw = undefined;
  }
}

function kittyData(base64: string, columns: number, rows: number): string {
  const chunks = base64.match(new RegExp(`.{1,${KITTY_CHUNK_SIZE}}`, "g")) ?? [];
  return chunks.map((chunk, index) => {
    const hasMore = index + 1 < chunks.length ? 1 : 0;
    if (index === 0) {
      return `${ESC}_Ga=T,t=d,f=100,c=${columns},r=${rows},q=2,i=${IMAGE_ID},m=${hasMore};${chunk}${STRING_TERMINATOR}`;
    }
    return `${ESC}_Gm=${hasMore};${chunk}${STRING_TERMINATOR}`;
  }).join("");
}

function kittyFile(pathBase64: string, columns: number, rows: number): string {
  return `${ESC}_Ga=T,t=f,f=100,c=${columns},r=${rows},q=2,i=${IMAGE_ID};${pathBase64}${STRING_TERMINATOR}`;
}

function kittyDelete(): string {
  return `${ESC}_Ga=d,d=I,i=${IMAGE_ID},q=2;${STRING_TERMINATOR}`;
}

function cursorTo(row: number, column: number): string {
  return `${ESC}[${row};${column}H`;
}

function saveCursor(): string {
  return `${ESC}7`;
}

function restoreCursor(): string {
  return `${ESC}8`;
}

function versionAtLeast(
  value: string | undefined,
  minimum: [number, number, number]
): boolean {
  if (!value) {
    return false;
  }
  const parts = value.split(".").map((part) => Number.parseInt(part, 10));
  if (parts.some((part) => !Number.isFinite(part)) || parts.length > 3) {
    return false;
  }
  const version: [number, number, number] = [
    parts[0] ?? 0,
    parts[1] ?? 0,
    parts[2] ?? 0
  ];
  for (let index = 0; index < 3; index += 1) {
    if (version[index]! !== minimum[index]!) {
      return version[index]! > minimum[index]!;
    }
  }
  return true;
}
