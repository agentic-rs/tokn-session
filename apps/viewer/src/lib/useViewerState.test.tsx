import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { eventButtonId } from "./state";
import { listSessions, loadEventDetail, loadEventPage } from "./tauri";
import type { EventDetail, EventPageResponse, SessionSummary } from "./types";
import { useViewerState } from "./useViewerState";

vi.mock("./tauri", () => ({
  listSessions: vi.fn(() => new Promise(() => undefined)),
  loadEventDetail: vi.fn(() => new Promise(() => undefined)),
  loadEventPage: vi.fn(() => new Promise(() => undefined)),
}));

beforeEach(() => {
  vi.mocked(listSessions).mockReset().mockImplementation(() => new Promise(() => undefined));
  vi.mocked(loadEventPage).mockReset().mockImplementation(() => new Promise(() => undefined));
  vi.mocked(loadEventDetail).mockReset().mockImplementation(() => new Promise(() => undefined));
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function session(sessionKey: string): SessionSummary {
  return {
    session_key: sessionKey,
    session_id: sessionKey,
    parent_session_id: null,
    provider: "codex",
    title: sessionKey,
    project: "viewer",
    cwd: "/work/repo",
    updated_at_ms: 1,
    timestamp: "2026-08-31T00:00:00Z",
    agent_path: null,
    message_count: null,
    event_count: 1,
    history_status: "complete",
  };
}

function toolEventPage(): EventPageResponse {
  return {
    events: [{
      event_key: "event.v1.1",
      type: "tool_call",
      provider: "codex",
      timestamp: "2026-08-31T00:00:00Z",
      phase: "finished",
      role: null,
      title: "exec_command",
      summary: "shell exit 0 cargo test",
      summary_truncated: false,
      is_hidden: false,
      is_error: false,
      tool: {
        kind: "shell",
        tool_name: "exec_command",
        tool_call_id: "call-1",
        command: "cargo test",
        cwd: "/work/repo",
        path: null,
        query: null,
        url: null,
        task_title: null,
        exit_code: 0,
        bytes: null,
        added: null,
        removed: null,
      },
    }],
    next_cursor: null,
    previous_cursor: null,
    total_events: 1,
    history_status: "complete",
  };
}

function toolDetail(text: string): EventDetail {
  return {
    event_key: "event.v1.1",
    event: { type: "tool_call" },
    native: null,
    is_hidden: false,
    tool_output: {
      sections: [{ label: "stdout", text, format: "text" }],
      truncated: false,
      original_size_bytes: text.length,
      source_event_key: "event.v1.1",
    },
  };
}

describe("useViewerState inspector focus", () => {
  it("restores focus to the selected event button after a pointer-style selection", () => {
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 1;
    });
    const eventKey = "event.v1.2";
    const trigger = document.createElement("button");
    trigger.id = eventButtonId(eventKey);
    document.body.append(trigger);
    const { result, unmount } = renderHook(() => useViewerState());

    expect(document.activeElement).toBe(document.body);
    act(() => result.current.selectEvent(eventKey));
    expect(result.current.inspectorOpen).toBe(true);
    act(() => result.current.closeInspector());

    expect(trigger).toHaveFocus();
    unmount();
    trigger.remove();
  });
});

describe("useViewerState expanded tool detail", () => {
  it("loads only after expansion and reuses the shared detail cache", async () => {
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [{
        session_key: "codex:session-1",
        session_id: "session-1",
        parent_session_id: null,
        provider: "codex",
        title: "Tool session",
        project: "viewer",
        cwd: "/work/repo",
        updated_at_ms: 1,
        timestamp: "2026-08-31T00:00:00Z",
        agent_path: null,
        message_count: null,
        event_count: 1,
        history_status: "complete",
      }],
      next_cursor: null,
      source_errors: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue({
      events: [{
        event_key: "event.v1.1",
        type: "tool_call",
        provider: "codex",
        timestamp: "2026-08-31T00:00:00Z",
        phase: "finished",
        role: null,
        title: "exec_command",
        summary: "shell exit 0 cargo test",
        summary_truncated: false,
        is_hidden: false,
        is_error: false,
        tool: {
          kind: "shell",
          tool_name: "exec_command",
          tool_call_id: "call-1",
          command: "cargo test",
          cwd: "/work/repo",
          path: null,
          query: null,
          url: null,
          task_title: null,
          exit_code: 0,
          bytes: null,
          added: null,
          removed: null,
        },
      }],
      next_cursor: null,
      previous_cursor: null,
      total_events: 1,
      history_status: "complete",
    });
    vi.mocked(loadEventDetail).mockResolvedValue({
      event_key: "event.v1.1",
      event: { type: "tool_call" },
      native: null,
      is_hidden: false,
      tool_output: {
        sections: [{ label: "stdout", text: "ok", format: "text" }],
        truncated: false,
        original_size_bytes: 2,
        source_event_key: "event.v1.1",
      },
    });
    const { result } = renderHook(() => useViewerState());

    await waitFor(() => expect(result.current.events).toHaveLength(1));
    expect(loadEventDetail).not.toHaveBeenCalled();

    act(() => result.current.toggleEventExpanded("event.v1.1"));
    await waitFor(() => expect(result.current.expandedDetail?.tool_output).not.toBeNull());
    expect(loadEventDetail).toHaveBeenCalledOnce();

    act(() => result.current.toggleEventExpanded("event.v1.1"));
    act(() => result.current.toggleEventExpanded("event.v1.1"));
    await waitFor(() => expect(result.current.expandedDetail?.tool_output).not.toBeNull());
    expect(loadEventDetail).toHaveBeenCalledOnce();
  });

  it("ignores stale cache writes without deleting a fresh request for the same session key", async () => {
    const oldA = deferred<EventDetail>();
    const freshA = deferred<EventDetail>();
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [session("codex:a"), session("codex:b")],
      next_cursor: null,
      source_errors: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue(toolEventPage());
    vi.mocked(loadEventDetail)
      .mockImplementationOnce(() => oldA.promise)
      .mockImplementationOnce(() => freshA.promise);
    const { result } = renderHook(() => useViewerState());

    await waitFor(() => expect(result.current.eventsOwnerKey).toBe("codex:a"));
    act(() => result.current.toggleEventExpanded("event.v1.1"));
    await waitFor(() => expect(loadEventDetail).toHaveBeenCalledTimes(1));

    act(() => result.current.selectSession("codex:b"));
    await waitFor(() => expect(result.current.eventsOwnerKey).toBe("codex:b"));
    act(() => result.current.selectSession("codex:a"));
    await waitFor(() => expect(result.current.eventsOwnerKey).toBe("codex:a"));
    act(() => result.current.toggleEventExpanded("event.v1.1"));
    await waitFor(() => expect(loadEventDetail).toHaveBeenCalledTimes(2));

    await act(async () => {
      oldA.resolve(toolDetail("stale A"));
      await oldA.promise;
    });
    expect(result.current.expandedDetail).toBeNull();
    expect(result.current.expandedDetailLoading).toBe(true);

    act(() => result.current.toggleEventExpanded("event.v1.1"));
    act(() => result.current.toggleEventExpanded("event.v1.1"));
    await waitFor(() => expect(result.current.expandedDetailLoading).toBe(true));
    expect(loadEventDetail).toHaveBeenCalledTimes(2);

    await act(async () => {
      freshA.resolve(toolDetail("fresh A"));
      await freshA.promise;
    });
    await waitFor(() => {
      expect(result.current.expandedDetail?.tool_output?.sections[0]?.text).toBe("fresh A");
    });

    act(() => result.current.toggleEventExpanded("event.v1.1"));
    act(() => result.current.toggleEventExpanded("event.v1.1"));
    await waitFor(() => {
      expect(result.current.expandedDetail?.tool_output?.sections[0]?.text).toBe("fresh A");
    });
    expect(loadEventDetail).toHaveBeenCalledTimes(2);
  });
});
