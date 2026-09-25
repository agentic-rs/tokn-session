import { describe, expect, it } from "vitest";
import type { SessionSummary } from "./types";
import { reconcileRecent, sidebarGroups } from "./sidebarOrder";

function session(session_key: string, updated_at_ms: number | null, overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    session_key, session_id: session_key, updated_at_ms, provider: "codex",
    parent_session_id: null, is_subagent: false, title: null, preview: null,
    project: null, cwd: null, timestamp: null, agent_path: null, agent_nickname: null,
    agent_role: null, child_count: 0, message_count: null, event_count: null,
    history_status: null, has_unread: false, ...overrides,
  };
}
const keys = (entries: { session_key: string }[]) => entries.map((entry) => entry.session_key);

describe("sidebar ordering", () => {
  it("freezes recent activity, prepends arrivals, and appends older page loads", () => {
    const now = Date.now();
    const a = session("a", now - 1000);
    const b = session("b", now - 2000);
    let ranks = reconcileRecent([], [b, a], now);
    expect(keys(ranks)).toEqual(["a", "b"]);
    ranks = reconcileRecent(ranks, [a, { ...b, updated_at_ms: now }, session("c", now), session("d", now - 3000)], now);
    expect(keys(ranks)).toEqual(["c", "a", "b", "d"]);
    expect(keys(reconcileRecent(ranks, [a], now))).toEqual(["c", "a", "b", "d"]);
    expect(reconcileRecent(ranks, [], now + 3600001)).toEqual([]);
  });

  it("uses exclusive local calendar buckets and handles missing timestamps", () => {
    const now = new Date(2026, 8, 25, 12).getTime();
    const hour = 3600000;
    const sessions = [session("recent", now), session("today", now - 2 * hour),
      session("yesterday", now - 24 * hour), session("week", now - 72 * hour),
      session("older", now - 8 * 24 * hour), session("unknown", null)];
    expect(sidebarGroups(sessions, "time", reconcileRecent([], sessions, now), now)
      .map((group) => [group.key, keys(group.sessions)]))
      .toEqual(sessions.map((entry) => [entry.session_key, [entry.session_key]]));
  });

  it("groups worktrees and subdirectories under the shared repository", () => {
    const sessions = [session("a", 500, { cwd: "/repo", project_key: "/repo", project_order_ms: 10 }),
      session("b", 100, { cwd: "/worktrees/task/repo", project_key: "/repo", project_order_ms: 10 }),
      session("c", 400, { cwd: "/repo/src", project_key: "/repo", project_order_ms: 10 })];
    const groups = sidebarGroups(sessions, "project", [], Date.now());
    expect(groups).toHaveLength(1);
    expect(keys(groups[0].sessions)).toEqual(["a", "c", "b"]);
  });

  it("orders projects by their durable anchor and keeps identical basenames distinct", () => {
    const sessions = [session("a", 500, { cwd: "/a/repo", project: "repo", project_order_ms: 10 }),
      session("b", 100, { cwd: "/b/repo", project: "repo", project_order_ms: 20 }),
      session("c", 400, { cwd: "/a/repo", project: "repo", project_order_ms: 10 })];
    expect(sidebarGroups(sessions, "project", [], Date.now()).map((group) => keys(group.sessions)))
      .toEqual([["b"], ["a", "c"]]);
  });
});
