import type { PixelFrame } from "./art";
import type { ImageAnchor, ImageProtocol } from "./image_protocol";
import type { JsonlStats } from "./jsonl";
import {
  effectivePetState,
  type PetFocus,
  type PetSnapshot,
  type PetState
} from "./state";

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
  control_mode?: "relay" | "demo" | "signal_only" | "none";
  focus_mode?: "auto" | "manual";
  input_active?: boolean;
  input_line?: string;
  input_status?: string;
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

interface SessionWindow {
  sessions: PetFocus[];
  hidden_before: number;
  hidden_after: number;
  hidden_urgent: number;
}

interface SessionFamily {
  root: PetFocus;
  sessions: PetFocus[];
  active_count: number;
  recent_count: number;
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

  const focusPanel = focusPanelLines(
    snapshot.focus,
    rightWidth,
    options.now_ms,
    options.color
  );
  const roster = rosterLines(
    snapshot,
    rightWidth,
    Math.max(1, contentRows - focusPanel.length),
    options.now_ms,
    options.color
  );
  const rightLines = [
    ...focusPanel,
    ...roster
  ];
  for (let index = 0; index < contentRows; index += 1) {
    lines.push(joinColumns(
      leftLines[index] ?? "",
      rightLines[index] ?? "",
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
    dim(truncate(controlLine(meta), contentWidth), options.color),
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
  lines.push(dim(truncate(controlLine(meta), contentWidth), options.color));
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
  const hiddenUrgent = snapshot.sessions.filter((session) => (
    session.topic !== focus?.topic && isUrgent(session)
  )).length;
  const summaryParts: string[] = [];
  if (hiddenUrgent > 0) {
    summaryParts.push(`! ${hiddenUrgent} urgent hidden`);
  }
  if (meta.focus_mode === "manual") {
    summaryParts.push("focus MANUAL");
  }
  summaryParts.push(options.name, `${snapshot.active_sessions}a`, `${recent}r`);
  const summary = summaryParts.join(" · ");
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
  const providerWidth = rosterProviderWidth(snapshot.sessions);
  const ageWidth = rosterAgeWidth(snapshot.sessions, nowMs);
  const families = sessionFamilies(snapshot);
  const activeFamilies = families.filter((family) => (
    family.active_count > 0 || family.root.family_state !== "idle"
  ));
  const recentFamilies = families.filter((family) => (
    family.active_count === 0
    && family.root.family_state === "idle"
    && family.recent_count > 0
  ));
  const full: RosterLine[] = [];

  appendRosterGroup(
    full,
    "ACTIVE",
    activeFamilies.flatMap((family) => family.sessions),
    activeFamilies.reduce((count, family) => count + family.active_count, 0),
    snapshot.focus?.topic,
    width,
    nowMs,
    color,
    providerWidth,
    ageWidth
  );
  appendRosterGroup(
    full,
    "RECENT",
    recentFamilies.flatMap((family) => family.sessions),
    recentFamilies.reduce((count, family) => count + family.recent_count, 0),
    snapshot.focus?.topic,
    width,
    nowMs,
    color,
    providerWidth,
    ageWidth
  );

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
      color,
      providerWidth,
      ageWidth
    ));
  }
  if (maxRows <= 1) {
    const focus = snapshot.focus ?? snapshot.sessions[0];
    return focus
      ? [
          sessionLine(
            focus,
            snapshot.focus?.topic,
            width,
            nowMs,
            color,
            providerWidth,
            ageWidth
          )
        ]
      : [];
  }

  const window = sessionWindow(snapshot, maxRows - 1);
  return [
    ...window.sessions.map((session) => sessionLine(
      session,
      snapshot.focus?.topic,
      width,
      nowMs,
      color,
      providerWidth,
      ageWidth
    )),
    dim(truncate(overflowLine(window), width), color)
  ];
}

function sessionFamilies(snapshot: PetSnapshot): SessionFamily[] {
  const families = new Map<string, PetFocus[]>();
  for (const session of snapshot.sessions) {
    const family = families.get(session.root_topic);
    if (family) {
      family.push(session);
    } else {
      families.set(session.root_topic, [session]);
    }
  }

  return [...families.entries()].map(([rootTopic, sessions]) => {
    const root = sessions.find((session) => session.topic === rootTopic)
      ?? sessions.find((session) => session.depth === 0)
      ?? sessions[0]!;
    return {
      root,
      sessions,
      active_count: sessions.filter((session) => session.state !== "idle").length,
      recent_count: sessions.filter(isRecentlyCompleted).length
    };
  });
}

function sessionWindow(
  snapshot: PetSnapshot,
  limit: number
): SessionWindow {
  const focusIndex = snapshot.focus
    ? snapshot.sessions.findIndex((session) => session.topic === snapshot.focus?.topic)
    : -1;
  const desiredStart = focusIndex < 0
    ? 0
    : focusIndex - Math.floor(limit / 2);
  const maxStart = Math.max(0, snapshot.sessions.length - limit);
  const start = Math.max(0, Math.min(desiredStart, maxStart));
  const sessions = snapshot.sessions.slice(start, start + limit);
  const hidden = [
    ...snapshot.sessions.slice(0, start),
    ...snapshot.sessions.slice(start + sessions.length)
  ];
  return {
    sessions,
    hidden_before: start,
    hidden_after: Math.max(0, snapshot.sessions.length - start - sessions.length),
    hidden_urgent: hidden.filter(isUrgent).length
  };
}

function overflowLine(window: SessionWindow): string {
  const prefix = window.hidden_urgent > 0
    ? `! ${window.hidden_urgent} urgent hidden · `
    : "";
  if (window.hidden_before > 0 && window.hidden_after > 0) {
    return `${prefix}… ${window.hidden_before} more above · ${window.hidden_after} below`;
  }
  if (window.hidden_before > 0) {
    return `${prefix}… ${window.hidden_before} more above`;
  }
  return `${prefix}… +${window.hidden_after} more`;
}

function controlLine(meta: RenderMeta): string {
  const mode = meta.control_mode ?? "relay";
  if (mode === "none") {
    return "Snapshot · no interactive controls";
  }
  if (mode === "signal_only") {
    return "Ctrl-C quit · keyboard controls unavailable";
  }
  if (meta.input_active) {
    return "Enter send · Esc cancel";
  }
  const focus = `focus ${(meta.focus_mode ?? "auto").toUpperCase()}`;
  const input = mode === "relay" ? " · Enter input" : "";
  const clear = mode === "relay" ? " · c clear" : "";
  return `${focus}${input} · ↑/↓ select · a auto${clear} · q/Esc quit`;
}

function isUrgent(session: PetFocus): boolean {
  return session.state === "needs_input" || session.state === "blocked";
}

function appendRosterGroup(
  target: RosterLine[],
  name: string,
  sessions: PetFocus[],
  count: number,
  focusTopic: string | undefined,
  width: number,
  nowMs: number,
  color: boolean,
  providerWidth: number,
  ageWidth: number
): void {
  if (sessions.length === 0) {
    return;
  }
  target.push({
    line: dim(truncate(`${name} ${count}`, width), color)
  });
  for (const session of sessions) {
    target.push({
      line: sessionLine(
        session,
        focusTopic,
        width,
        nowMs,
        color,
        providerWidth,
        ageWidth
      )
    });
  }
}

function sessionLine(
  session: PetFocus,
  focusTopic: string | undefined,
  width: number,
  nowMs: number,
  color: boolean,
  providerWidth = 0,
  ageWidth = 0
): string {
  const rowState = effectivePetState(session);
  const isRecent = rowState === "idle" && session.recently_completed;
  const isInterrupted = isRecent && session.outcome === "interrupted";
  const displayState = isRecent && !isInterrupted ? "ready" : rowState;
  const marker = session.topic === focusTopic ? "›" : " ";
  const glyph = isInterrupted ? "×" : isRecent ? "✓" : STATUS_GLYPH[rowState];
  const provider = providerLabel(session);
  const age = sessionAge(session, nowMs);
  const alignedProvider = padRight(
    provider,
    Math.max(providerWidth, Bun.stringWidth(provider))
  );
  const prefix = width >= 32
    ? `${marker} ${glyph} ${alignedProvider} · `
    : `${marker} ${glyph} `;
  const suffix = ` · ${padLeft(age, Math.max(ageWidth, Bun.stringWidth(age)))}`;
  const separator = " · ";
  const separatorWidth = Bun.stringWidth(separator);
  const available = width
    - Bun.stringWidth(prefix)
    - Bun.stringWidth(suffix);
  if (available < separatorWidth + 2) {
    const compact = [
      `${marker} ${glyph} ${provider} · ${age}`,
      `${marker} ${glyph} ${age}`,
      `${glyph} ${provider} · ${age}`,
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
  const identityRatio = width < 32 ? 0.25 : 0.5;
  const identityWidth = Math.max(
    1,
    width < 32
      ? Math.floor(contentWidth * identityRatio)
      : Math.ceil(contentWidth * identityRatio)
  );
  const activityWidth = Math.max(1, contentWidth - identityWidth);
  const identity = indentedSessionIdentity(session, identityWidth);
  const plain = [
    prefix,
    padRight(identity, identityWidth),
    separator,
    padRight(truncate(sessionActivity(session), activityWidth), activityWidth),
    suffix
  ].join("");
  return colorize(plain, color, STATUS_COLOR[displayState]);
}

function rosterProviderWidth(sessions: PetFocus[]): number {
  return sessions.reduce((width, session) => (
    Math.max(width, Bun.stringWidth(providerLabel(session)))
  ), 0);
}

function rosterAgeWidth(sessions: PetFocus[], nowMs: number): number {
  return sessions.reduce((width, session) => (
    Math.max(width, Bun.stringWidth(sessionAge(session, nowMs)))
  ), 0);
}

function sessionAge(session: PetFocus, nowMs: number): string {
  const lastEventAt = session.depth === 0
    ? session.family_last_event_at
    : session.last_event_at;
  const timestamp = effectivePetState(session) === "idle" && session.recently_completed
    ? session.completed_at ?? lastEventAt
    : lastEventAt;
  return formatAge(Math.max(0, nowMs - timestamp));
}

function indentedSessionIdentity(session: PetFocus, width: number): string {
  const indent = session.depth > 0
    ? `${"  ".repeat(Math.min(session.depth - 1, 4))}↳ `
    : "";
  const indentWidth = Bun.stringWidth(indent);
  if (indentWidth >= width) {
    return truncate(indent.trimStart(), width);
  }
  return `${indent}${truncate(sessionIdentity(session), width - indentWidth)}`;
}

function focusStatusLine(
  focus: PetFocus | undefined,
  fallbackState: PetState,
  color: boolean
): string {
  const state = focus ? effectivePetState(focus) : fallbackState;
  if (focus && state === "idle" && focus.recently_completed) {
    if (focus.outcome === "interrupted") {
      return colorize(`${BOLD}× Interrupted${RESET}`, color, STATUS_COLOR.idle);
    }
    return colorize(`${BOLD}✓ Ready recently${RESET}`, color, STATUS_COLOR.ready);
  }
  return statusLine(state, color);
}

function focusPanelLines(
  focus: PetFocus | undefined,
  width: number,
  nowMs: number,
  color: boolean
): string[] {
  if (!focus) {
    return [
      dim(truncate("FOCUS · Waiting for Relay activity", width), color),
      ""
    ];
  }

  const state = effectivePetState(focus);
  const activity = focus.current_activity?.detail
    ?? focus.current_activity?.label
    ?? focus.label;
  const detail = `${focusReason(focus, state)}: ${activity}`;
  const location = focus.cwd
    ? `cwd ${focus.cwd}`
    : contextLine(focus);
  const metadata = [
    formatAgeLabel(sessionAge(focus, nowMs)),
    location,
    focus.depth === 0 && focus.descendant_count > 0
      ? sessionActivity(focus)
      : undefined
  ].filter((value): value is string => Boolean(value)).join(" · ");

  return [
    colorize(
      `${BOLD}FOCUS${RESET} · ${truncate(sessionIdentity(focus), Math.max(1, width - 8))}`,
      color,
      STATUS_COLOR[state]
    ),
    truncate(detail, width),
    dim(truncate(metadata, width), color),
    ""
  ];
}

function focusReason(focus: PetFocus, state: PetState): string {
  if (state === "idle" && focus.recently_completed) {
    return focus.outcome === "interrupted" ? "Interrupted" : "Last result";
  }
  switch (state) {
    case "needs_input":
      return "Waiting for input";
    case "blocked":
      return "Blocked";
    case "ready":
      return "Latest result";
    case "running":
      return "Current activity";
    case "idle":
      return "Last activity";
  }
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
    focus.project_label,
    normalizeAgent(focus.agent)
  ].filter((value): value is string => Boolean(value));
  return values.join(" · ");
}

function sessionIdentity(session: PetFocus): string {
  const shortId = session.session_id.length <= 8
    ? session.session_id
    : session.session_id.slice(0, 8);
  const agent = normalizeAgent(session.agent);
  const identity = session.depth > 0
    ? agent || session.title || "agent"
    : session.project_label || session.title || "session";
  return `${identity} · ${shortId}`;
}

function providerLabel(session: PetFocus): string {
  const provider = session.provider ?? session.topic.split(".", 1)[0];
  return provider?.toLowerCase() || "unknown";
}

function sessionActivity(session: PetFocus): string {
  if (session.depth !== 0 || session.descendant_count === 0) {
    return session.label;
  }

  const parts = [formatCount(session.descendant_count, "agent")];
  if (session.urgent_descendant_count > 0) {
    parts.push(`${session.urgent_descendant_count} urgent`);
  } else if (session.active_descendant_count > 0) {
    parts.push(`${session.active_descendant_count} active`);
  } else if (session.recent_descendant_count > 0) {
    parts.push(`${session.recent_descendant_count} recent`);
  }
  if (session.state !== "idle") {
    parts.push(session.label);
  }
  return parts.join(" · ");
}

function formatCount(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

function formatAgeLabel(age: string): string {
  return age === "now" ? age : `${age} ago`;
}

function normalizeAgent(agent: string | undefined): string | undefined {
  const normalized = agent?.replace(/^\/+/, "");
  return normalized || undefined;
}

function recentSessions(snapshot: PetSnapshot): PetFocus[] {
  return snapshot.sessions.filter(isRecentlyCompleted);
}

function isRecentlyCompleted(session: PetFocus): boolean {
  return session.state === "idle" && session.recently_completed;
}

function activityLine(snapshot: PetSnapshot, meta: RenderMeta): string {
  if (meta.input_active) {
    return `> ${meta.input_line ?? ""}`;
  }
  if (meta.input_status) {
    return meta.input_status;
  }
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

function padLeft(value: string, width: number): string {
  const visibleWidth = Bun.stringWidth(value);
  if (visibleWidth >= width) {
    return visibleWidth === width ? value : truncate(Bun.stripANSI(value), width);
  }
  return `${" ".repeat(width - visibleWidth)}${value}`;
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
