import type { PixelFrame } from "./art";
import type { ImageAnchor, ImageProtocol } from "./image_protocol";
import type { JsonlStats } from "./jsonl";
import type { PetSnapshot, PetState } from "./state";

const RESET = "\u001b[0m";
const DIM = "\u001b[2m";
const BOLD = "\u001b[1m";

const STATUS_LABEL: Record<PetState, string> = {
  idle: "Idle",
  running: "Running",
  needs_input: "Needs input",
  ready: "Ready",
  blocked: "Blocked"
};

const STATUS_COLOR: Record<PetState, [number, number, number]> = {
  idle: [139, 148, 158],
  running: [74, 158, 255],
  needs_input: [245, 184, 65],
  ready: [83, 199, 127],
  blocked: [239, 91, 91]
};

export interface RenderMeta {
  source_label: string;
  diagnostic?: string;
  stats?: JsonlStats;
}

export interface RenderOptions {
  columns: number;
  rows: number;
  color: boolean;
  image_protocol: ImageProtocol;
  name: string;
}

export interface RenderedScreen {
  lines: string[];
  image_anchor?: ImageAnchor;
}

export function renderScreen(
  snapshot: PetSnapshot,
  ansiFrame: PixelFrame,
  meta: RenderMeta,
  options: RenderOptions
): RenderedScreen {
  const columns = Math.max(1, options.columns);
  const rows = Math.max(1, options.rows);
  const compact = rows < 13 || columns < 28;
  const body = compact
    ? renderCompact(snapshot, meta, options)
    : renderFull(snapshot, ansiFrame, meta, options);
  const visibleBody = body.lines.slice(0, rows);
  const top = Math.max(0, Math.floor((rows - visibleBody.length) / 2));
  const lines = Array.from({ length: rows }, () => "");

  visibleBody.forEach((line, index) => {
    lines[top + index] = center(line, columns);
  });

  if (!body.image_anchor) {
    return { lines };
  }
  return {
    lines,
    image_anchor: {
      ...body.image_anchor,
      row: top + body.image_anchor.row
    }
  };
}

function renderFull(
  snapshot: PetSnapshot,
  ansiFrame: PixelFrame,
  meta: RenderMeta,
  options: RenderOptions
): RenderedScreen {
  const lines: string[] = [];
  const contentWidth = Math.max(1, Math.min(58, options.columns - 2));
  const name = truncate(options.name.toUpperCase().split("").join(" "), contentWidth);
  lines.push(colorize(`${BOLD}${name}${RESET}`, options.color));
  lines.push("");

  let imageAnchor: ImageAnchor | undefined;
  if (options.image_protocol === "ansi") {
    lines.push(...renderAnsiFrame(ansiFrame, options.color));
  } else {
    const imageRows = 5;
    imageAnchor = {
      column: Math.max(1, Math.floor((options.columns - 10) / 2) + 1),
      row: lines.length + 1,
      columns: 10,
      rows: imageRows
    };
    for (let index = 0; index < imageRows; index += 1) {
      lines.push("");
    }
  }

  lines.push("");
  lines.push(statusLine(snapshot.state, options.color));
  const context = contextLine(snapshot);
  if (context) {
    lines.push(dim(truncate(context, contentWidth), options.color));
  }
  lines.push(truncate(snapshot.focus?.label ?? "Waiting for Relay activity", contentWidth));
  lines.push(dim(truncate(activityLine(snapshot, meta), contentWidth), options.color));
  if (meta.diagnostic) {
    lines.push(colorize(truncate(meta.diagnostic, contentWidth), options.color, [239, 91, 91]));
  }
  lines.push("");
  lines.push(dim(truncate("q quit  ·  c clear notifications", contentWidth), options.color));

  return {
    lines,
    image_anchor: imageAnchor
  };
}

function renderCompact(
  snapshot: PetSnapshot,
  meta: RenderMeta,
  options: RenderOptions
): RenderedScreen {
  const contentWidth = Math.max(1, options.columns - 2);
  const name = truncate(options.name, Math.max(1, Math.floor(contentWidth / 2)));
  return {
    lines: [
      `${name}  ${statusLine(snapshot.state, options.color)}`,
      truncate(snapshot.focus?.label ?? "Waiting for Relay activity", contentWidth),
      dim(truncate(activityLine(snapshot, meta), contentWidth), options.color)
    ]
  };
}

export function renderAnsiFrame(frame: PixelFrame, color: boolean): string[] {
  const lines: string[] = [];
  for (let y = 0; y < frame.length; y += 2) {
    const upper = frame[y] ?? [];
    const lower = frame[y + 1] ?? [];
    const width = Math.max(upper.length, lower.length);
    let line = "";

    for (let x = 0; x < width; x += 1) {
      const top = upper[x] ?? null;
      const bottom = lower[x] ?? null;
      if (!top && !bottom) {
        line += " ";
      } else if (!color) {
        line += top && bottom ? "█" : top ? "▀" : "▄";
      } else if (top && bottom) {
        line += `${foreground(top)}${background(bottom)}▀${RESET}`;
      } else if (top) {
        line += `${foreground(top)}▀${RESET}`;
      } else if (bottom) {
        line += `${foreground(bottom)}▄${RESET}`;
      }
    }
    lines.push(line.trimEnd());
  }
  return lines;
}

function statusLine(state: PetState, color: boolean): string {
  const label = `● ${STATUS_LABEL[state]}`;
  return colorize(`${BOLD}${label}${RESET}`, color, STATUS_COLOR[state]);
}

function contextLine(snapshot: PetSnapshot): string {
  const values = [
    snapshot.focus?.provider,
    snapshot.focus?.project,
    snapshot.focus?.agent
  ].filter((value): value is string => Boolean(value));
  return values.join(" · ");
}

function activityLine(snapshot: PetSnapshot, meta: RenderMeta): string {
  const malformed = meta.stats?.malformed_lines ?? 0;
  const active = `${snapshot.active_sessions} active`;
  const total = `${snapshot.total_sessions} seen`;
  const parseStatus = malformed > 0 ? ` · ${malformed} malformed` : "";
  return `${active} · ${total} · ${meta.source_label}${parseStatus}`;
}

function center(value: string, width: number): string {
  const visibleWidth = Bun.stringWidth(value);
  const padding = Math.max(0, Math.floor((width - visibleWidth) / 2));
  return `${" ".repeat(padding)}${value}`;
}

function truncate(value: string, width: number): string {
  const singleLine = sanitizeTerminalText(value).replaceAll(/\s+/g, " ").trim();
  if (Bun.stringWidth(singleLine) <= width) {
    return singleLine;
  }
  if (width <= 1) {
    return width === 1 ? "…" : "";
  }

  const targetWidth = width - 1;
  const segments = new Intl.Segmenter(undefined, {
    granularity: "grapheme"
  }).segment(singleLine);
  let result = "";
  let resultWidth = 0;
  for (const { segment } of segments) {
    const segmentWidth = Bun.stringWidth(segment);
    if (resultWidth + segmentWidth > targetWidth) {
      break;
    }
    result += segment;
    resultWidth += segmentWidth;
  }
  return `${result}…`;
}

function colorize(
  value: string,
  enabled: boolean,
  color: [number, number, number] = [238, 238, 238]
): string {
  return enabled
    ? `\u001b[38;2;${color[0]};${color[1]};${color[2]}m${value}${RESET}`
    : Bun.stripANSI(value);
}

function dim(value: string, enabled: boolean): string {
  return enabled ? `${DIM}${value}${RESET}` : value;
}

function foreground(pixel: { red: number; green: number; blue: number }): string {
  return `\u001b[38;2;${pixel.red};${pixel.green};${pixel.blue}m`;
}

function background(pixel: { red: number; green: number; blue: number }): string {
  return `\u001b[48;2;${pixel.red};${pixel.green};${pixel.blue}m`;
}

function sanitizeTerminalText(value: string): string {
  return Bun
    .stripANSI(value)
    .replaceAll(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g, "");
}
