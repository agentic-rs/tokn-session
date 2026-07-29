import type { PixelFrame } from "./art";
import type { ImageAnchor, ImageProtocol } from "./image_protocol";
import type { JsonlStats } from "./jsonl";
import type { PetFocus, PetSnapshot, PetState } from "./state";

const RESET = "\u001b[0m";
const DIM = "\u001b[2m";
const BOLD = "\u001b[1m";
const WIDE_COLUMNS = 56;
const WIDE_ROWS = 16;
const LIST_COLUMNS = 8;
const LIST_ROWS = 5;

const STATUS_LABEL: Record<PetState, string> = {
  idle: "Idle",
  running: "Running",
  needs_input: "Needs input",
  ready: "Ready",
  blocked: "Blocked"
};

const STATUS_GLYPH: Record<PetState, string> = {
  idle: "·",
  running: "●",
  needs_input: "?",
  ready: "✓",
  blocked: "!"
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
  now_ms?: number;
}

export interface RenderedScreen {
  lines: string[];
  image_anchor?: ImageAnchor;
}

interface RenderBody {
  lines: string[];
  image_anchor?: ImageAnchor;
}

interface RosterLine {
  line: string;
}

export function renderScreen(
  snapshot: PetSnapshot,
  ansiFrame: PixelFrame,
  meta: RenderMeta,
  options: RenderOptions
): RenderedScreen {
  const columns = Math.max(1, options.columns);
  const rows = Math.max(1, options.rows);
  const normalizedOptions = {
    ...options,
    columns,
    rows,
    now_ms: options.now_ms ?? Date.now()
  };
  const body = columns >= WIDE_COLUMNS && rows >= WIDE_ROWS
    ? renderWide(snapshot, ansiFrame, meta, normalizedOptions)
    : columns >= LIST_COLUMNS && rows >= LIST_ROWS
      ? renderList(snapshot, meta, normalizedOptions)
      : renderTiny(snapshot, meta, normalizedOptions);
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

function renderWide(
  snapshot: PetSnapshot,
  ansiFrame: PixelFrame,
  meta: RenderMeta,
  options: RenderOptions & { now_ms: number }
): RenderBody {
  const contentWidth = Math.max(1, Math.min(92, options.columns - 2));
  const leftWidth = 18;
  const gapWidth = 2;
  const rightWidth = Math.max(1, contentWidth - leftWidth - gapWidth);
  const footerRows = meta.diagnostic ? 3 : 2;
  const contentRows = Math.max(6, options.rows - footerRows - 2);
  const name = truncate(options.name.toUpperCase().split("").join(" "), leftWidth);
  const header = joinColumns(
    colorize(`${BOLD}${name}${RESET}`, options.color),
    dim(truncate("SESSION ROSTER", rightWidth), options.color),
    leftWidth,
    rightWidth,
    gapWidth
  );
  const lines = [header, " ".repeat(contentWidth)];
  const leftLines: string[] = [];
  let imageAnchor: ImageAnchor | undefined;

  if (options.image_protocol === "ansi") {
    leftLines.push(...renderAnsiFrame(ansiFrame, options.color));
  } else {
    const imageRows = 5;
    const imageColumns = 10;
    for (let index = 0; index < imageRows; index += 1) {
      leftLines.push("");
    }
    const leftOffset = Math.max(0, Math.floor((options.columns - contentWidth) / 2));
    imageAnchor = {
      column: leftOffset + Math.floor((leftWidth - imageColumns) / 2) + 1,
      row: 3,
      columns: imageColumns,
      rows: imageRows
    };
  }

  leftLines.push("");
  leftLines.push(focusStatusLine(snapshot.focus, snapshot.state, options.color));
  const context = contextLine(snapshot.focus);
  if (context) {
    leftLines.push(dim(truncate(context, leftWidth), options.color));
  }
  leftLines.push(truncate(
    snapshot.focus?.label ?? "Waiting for Relay activity",
    leftWidth
  ));

  const roster = rosterLines(
    snapshot,
    rightWidth,
    contentRows,
    options.now_ms,
    options.color
  );
  for (let index = 0; index < contentRows; index += 1) {
    lines.push(joinColumns(
      leftLines[index] ?? "",
      roster[index] ?? "",
      leftWidth,
      rightWidth,
      gapWidth
    ));
  }

  if (meta.diagnostic) {
    lines.push(padRight(
      colorize(
        truncate(meta.diagnostic, contentWidth),
        options.color,
        STATUS_COLOR.blocked
      ),
      contentWidth
    ));
  }
  lines.push(padRight(
    dim(truncate(activityLine(snapshot, meta), contentWidth), options.color),
    contentWidth
  ));
  lines.push(padRight(
    dim(truncate("q quit  ·  c clear focused notification", contentWidth), options.color),
    contentWidth
  ));

  return {
    lines,
    image_anchor: imageAnchor
  };
}

function renderList(
  snapshot: PetSnapshot,
  meta: RenderMeta,
  options: RenderOptions & { now_ms: number }
): RenderBody {
  const contentWidth = Math.max(1, options.columns - 2);
  const recent = recentSessions(snapshot).length;
  const headline = [
    options.name,
    `${snapshot.active_sessions} active`,
    `${recent} recent`
  ].join(" · ");
  const footerRows = meta.diagnostic ? 3 : 2;
  const rosterBudget = Math.max(1, options.rows - footerRows - 1);
  const lines = [
    colorize(`${BOLD}${truncate(headline, contentWidth)}${RESET}`, options.color),
    ...rosterLines(
      snapshot,
      contentWidth,
      rosterBudget,
      options.now_ms,
      options.color
    )
  ];
  if (meta.diagnostic) {
    lines.push(colorize(
      truncate(meta.diagnostic, contentWidth),
      options.color,
      STATUS_COLOR.blocked
    ));
  }
  lines.push(dim(truncate(activityLine(snapshot, meta), contentWidth), options.color));
  lines.push(dim(
    truncate("q quit  ·  c clear focused notification", contentWidth),
    options.color
  ));
  return { lines };
}

function renderTiny(
  snapshot: PetSnapshot,
  meta: RenderMeta,
  options: RenderOptions & { now_ms: number }
): RenderBody {
  const contentWidth = Math.max(1, options.columns);
  const recent = recentSessions(snapshot).length;
  const focus = snapshot.focus;
  const summary = `${options.name} · ${snapshot.active_sessions}a · ${recent}r`;
  const lines = [
    colorize(`${BOLD}${truncate(summary, contentWidth)}${RESET}`, options.color),
    focus
      ? sessionLine(focus, focus.topic, contentWidth, options.now_ms, options.color)
      : truncate("Waiting for Relay activity", contentWidth),
    dim(
      truncate(
        `${focus ? 1 : 0} shown · ${snapshot.sessions.length} roster · ${snapshot.total_sessions} seen`,
        contentWidth
      ),
      options.color
    )
  ];
  if (meta.diagnostic) {
    lines.unshift(colorize(
      truncate(meta.diagnostic, contentWidth),
      options.color,
      STATUS_COLOR.blocked
    ));
  }
  return {
    lines
  };
}

function rosterLines(
  snapshot: PetSnapshot,
  width: number,
  maxRows: number,
  nowMs: number,
  color: boolean
): string[] {
  const active = snapshot.sessions.filter((session) => session.state !== "idle");
  const recent = recentSessions(snapshot);
  const full: RosterLine[] = [];

  appendRosterGroup(full, "ACTIVE", active, snapshot.focus?.topic, width, nowMs, color);
  appendRosterGroup(full, "RECENT READY", recent, snapshot.focus?.topic, width, nowMs, color);

  if (full.length === 0) {
    return [dim(truncate("No active or recent sessions", width), color)];
  }
  if (full.length <= maxRows) {
    return full.map((entry) => entry.line);
  }
  if (snapshot.sessions.length <= maxRows) {
    return snapshot.sessions.map((session) => sessionLine(
      session,
      snapshot.focus?.topic,
      width,
      nowMs,
      color
    ));
  }
  if (maxRows <= 1) {
    return [dim(truncate(`… +${snapshot.sessions.length} more`, width), color)];
  }

  const visible = snapshot.sessions.slice(0, maxRows - 1);
  const hidden = Math.max(0, snapshot.sessions.length - visible.length);
  return [
    ...visible.map((session) => sessionLine(
      session,
      snapshot.focus?.topic,
      width,
      nowMs,
      color
    )),
    dim(truncate(`… +${hidden} more`, width), color)
  ];
}

function appendRosterGroup(
  target: RosterLine[],
  name: string,
  sessions: PetFocus[],
  focusTopic: string | undefined,
  width: number,
  nowMs: number,
  color: boolean
): void {
  if (sessions.length === 0) {
    return;
  }
  target.push({
    line: dim(truncate(`${name} ${sessions.length}`, width), color)
  });
  for (const session of sessions) {
    target.push({
      line: sessionLine(session, focusTopic, width, nowMs, color)
    });
  }
}

function sessionLine(
  session: PetFocus,
  focusTopic: string | undefined,
  width: number,
  nowMs: number,
  color: boolean
): string {
  const isRecent = session.state === "idle" && session.recently_completed;
  const displayState = isRecent ? "ready" : session.state;
  const marker = session.topic === focusTopic ? "›" : " ";
  const glyph = isRecent ? "✓" : STATUS_GLYPH[session.state];
  const status = isRecent ? "Ready" : STATUS_LABEL[session.state];
  const timestamp = isRecent
    ? session.completed_at ?? session.last_event_at
    : session.last_event_at;
  const age = formatAge(Math.max(0, nowMs - timestamp));
  const prefix = width >= 32
    ? `${marker} ${glyph} ${status} · `
    : `${marker} ${glyph} `;
  const suffix = ` · ${age}`;
  const separator = " · ";
  const separatorWidth = Bun.stringWidth(separator);
  const available = width - Bun.stringWidth(prefix) - Bun.stringWidth(suffix);
  if (available < separatorWidth + 2) {
    const compact = [
      `${marker} ${glyph} ${age}`,
      `${glyph} ${age}`,
      age
    ].find((candidate) => Bun.stringWidth(candidate.trim()) <= width) ?? age;
    return colorize(
      truncate(compact, width),
      color,
      STATUS_COLOR[displayState]
    );
  }
  const contentWidth = Math.max(1, available - separatorWidth);
  const identityRatio = width < 32 ? 0.25 : 0.47;
  const identityWidth = Math.max(1, Math.floor(contentWidth * identityRatio));
  const activityWidth = Math.max(1, contentWidth - identityWidth);
  const plain = [
    prefix,
    truncate(sessionIdentity(session), identityWidth),
    separator,
    truncate(session.label, activityWidth),
    suffix
  ].join("");
  return colorize(truncate(plain, width), color, STATUS_COLOR[displayState]);
}

function focusStatusLine(
  focus: PetFocus | undefined,
  fallbackState: PetState,
  color: boolean
): string {
  if (focus?.state === "idle" && focus.recently_completed) {
    return colorize(`${BOLD}✓ Ready recently${RESET}`, color, STATUS_COLOR.ready);
  }
  return statusLine(focus?.state ?? fallbackState, color);
}

function statusLine(state: PetState, color: boolean): string {
  const label = `${STATUS_GLYPH[state]} ${STATUS_LABEL[state]}`;
  return colorize(`${BOLD}${label}${RESET}`, color, STATUS_COLOR[state]);
}

function contextLine(focus: PetFocus | undefined): string {
  if (!focus) {
    return "";
  }
  const values = [
    focus.provider,
    focus.project,
    normalizeAgent(focus.agent)
  ].filter((value): value is string => Boolean(value));
  return values.join(" · ");
}

function sessionIdentity(session: PetFocus): string {
  const shortId = session.session_id.length <= 8
    ? session.session_id
    : session.session_id.slice(0, 8);
  const projectAgent = [
    session.project,
    normalizeAgent(session.agent)
  ].filter((value): value is string => Boolean(value)).join("/");
  const identity = session.title
    || projectAgent
    || session.provider
    || "session";
  return `${identity} · ${shortId}`;
}

function normalizeAgent(agent: string | undefined): string | undefined {
  const normalized = agent?.replace(/^\/+/, "");
  return normalized || undefined;
}

function recentSessions(snapshot: PetSnapshot): PetFocus[] {
  return snapshot.sessions.filter(
    (session) => session.state === "idle" && session.recently_completed
  );
}

function activityLine(snapshot: PetSnapshot, meta: RenderMeta): string {
  const malformed = meta.stats?.malformed_lines ?? 0;
  const active = `${snapshot.active_sessions} active`;
  const recent = `${recentSessions(snapshot).length} recent`;
  const total = `${snapshot.total_sessions} seen`;
  const parseStatus = malformed > 0 ? ` · ${malformed} malformed` : "";
  return `${active} · ${recent} · ${total} · ${meta.source_label}${parseStatus}`;
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

function formatAge(ageMs: number): string {
  if (ageMs < 5_000) {
    return "now";
  }
  if (ageMs < 60_000) {
    return `${Math.floor(ageMs / 1_000)}s`;
  }
  if (ageMs < 60 * 60_000) {
    return `${Math.floor(ageMs / 60_000)}m`;
  }
  return `${Math.floor(ageMs / (60 * 60_000))}h`;
}

function joinColumns(
  left: string,
  right: string,
  leftWidth: number,
  rightWidth: number,
  gapWidth: number
): string {
  return `${padRight(left, leftWidth)}${" ".repeat(gapWidth)}${padRight(right, rightWidth)}`;
}

function padRight(value: string, width: number): string {
  const visibleWidth = Bun.stringWidth(value);
  if (visibleWidth >= width) {
    return visibleWidth === width ? value : truncate(Bun.stripANSI(value), width);
  }
  return `${value}${" ".repeat(width - visibleWidth)}`;
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
