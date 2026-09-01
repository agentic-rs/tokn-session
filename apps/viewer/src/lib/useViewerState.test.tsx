import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { eventButtonId } from "./state";
import {
  acknowledgeSessionAttention,
  getSessionIndexProgress,
  listSessionChildren,
  listSessions,
  listenForSessionIndexChanges,
  listenForSessionIndexProgress,
  loadEventDetail,
  loadEventPage,
  loadTrajectoryEventPage,
  retrySessionIndex,
} from "./tauri";
import type {
  EventDetail,
  EventPageResponse,
  SessionIndexChangedEvent,
  SessionIndexProgress,
  SessionSummary,
  TrajectoryEventPageResponse,
} from "./types";
import { useViewerState } from "./useViewerState";

vi.mock("./tauri", () => ({
  acknowledgeSessionAttention: vi.fn(() => Promise.resolve({ changed: false })),
  getSessionIndexProgress: vi.fn(() => new Promise(() => undefined)),
  listSessionChildren: vi.fn(() => new Promise(() => undefined)),
  listSessions: vi.fn(() => new Promise(() => undefined)),
  listenForSessionIndexChanges: vi.fn(() => Promise.resolve(vi.fn())),
  listenForSessionIndexProgress: vi.fn(() => Promise.resolve(vi.fn())),
  loadEventDetail: vi.fn(() => new Promise(() => undefined)),
  loadEventPage: vi.fn(() => new Promise(() => undefined)),
  loadTrajectoryEventPage: vi.fn(() => new Promise(() => undefined)),
  retrySessionIndex: vi.fn(() => new Promise(() => undefined)),
}));

beforeEach(() => {
  vi.mocked(acknowledgeSessionAttention).mockReset().mockResolvedValue({ changed: false });
  vi.mocked(getSessionIndexProgress).mockReset().mockImplementation(() => new Promise(() => undefined));
  vi.mocked(listSessionChildren).mockReset().mockImplementation(() => new Promise(() => undefined));
  vi.mocked(listSessions).mockReset().mockImplementation(() => new Promise(() => undefined));
  vi.mocked(listenForSessionIndexChanges).mockReset().mockResolvedValue(vi.fn());
  vi.mocked(listenForSessionIndexProgress).mockReset().mockResolvedValue(vi.fn());
  vi.mocked(loadEventPage).mockReset().mockImplementation(() => new Promise(() => undefined));
  vi.mocked(loadEventDetail).mockReset().mockImplementation(() => new Promise(() => undefined));
  vi.mocked(loadTrajectoryEventPage).mockReset().mockImplementation(
    () => new Promise(() => undefined),
  );
  vi.mocked(retrySessionIndex).mockReset().mockImplementation(() => new Promise(() => undefined));
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

async function selectListedSession(
  result: { current: { sessions: SessionSummary[]; selectSession: (sessionKey: string) => void } },
  sessionKey: string,
) {
  await waitFor(() => {
    expect(result.current.sessions.some((session) => session.session_key === sessionKey)).toBe(true);
  });
  act(() => result.current.selectSession(sessionKey));
}

function session(sessionKey: string): SessionSummary {
  return {
    session_key: sessionKey,
    session_id: sessionKey,
    parent_session_id: null,
    is_subagent: false,
    provider: "codex",
    title: sessionKey,
    preview: null,
    project: "viewer",
    cwd: "/work/repo",
    updated_at_ms: 1,
    timestamp: "2026-08-31T00:00:00Z",
    agent_path: null,
    agent_nickname: null,
    agent_role: null,
    child_count: 0,
    message_count: null,
    event_count: 1,
    history_status: "complete",
    has_unread: false,
  };
}

function indexProgress(overrides: Partial<SessionIndexProgress> = {}): SessionIndexProgress {
  return {
    revision: "1",
    is_refreshing: false,
    activity: "idle",
    catalog: {
      active_provider: null,
      processed_providers: 6,
      total_providers: 6,
      pending_providers: [],
      error_providers: [],
    },
    body: {
      active_provider: null,
      pending_jobs: 0,
      failed_jobs: 0,
      completed_in_run: 0,
      stale_in_run: 0,
      batch_size: 1,
      providers: [],
    },
    worker_error: null,
    retry_at_ms: null,
    ...overrides,
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
      usage: null,
      reasoning: null,
    }],
    next_cursor: null,
    previous_cursor: null,
    total_events: 1,
    history_status: "complete",
  };
}

function pendingToolEventPage(): EventPageResponse {
  const page = toolEventPage();
  const event = page.events[0]!;
  return {
    ...page,
    events: [{
      ...event,
      phase: "started",
      summary: "shell running cargo test",
      tool: {
        ...event.tool!,
        exit_code: null,
      },
    }],
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

function reasoningEventPage(overrides: Partial<EventPageResponse["events"][number]> = {}): EventPageResponse {
  return {
    events: [{
      event_key: "event.v1.reasoning",
      type: "reasoning",
      provider: "codex",
      timestamp: "2026-08-31T00:00:00Z",
      phase: "finished",
      role: null,
      title: "Reasoning",
      summary: "Inspect the source",
      summary_truncated: false,
      is_hidden: false,
      is_error: false,
      tool: null,
      usage: null,
      reasoning: {
        preview: "Inspect the source",
        has_summary: true,
        has_text: false,
        has_encrypted_content: false,
        is_redacted: false,
      },
      ...overrides,
    }],
    next_cursor: null,
    previous_cursor: null,
    total_events: 1,
    history_status: "complete",
  };
}

function trajectoryEventPage(): EventPageResponse {
  return {
    events: [{
      event_key: "trajectory.v1.turn-1",
      type: "trajectory",
      provider: "codex",
      timestamp: "2026-08-31T01:00:00Z",
      phase: null,
      role: null,
      title: "Turn trajectory",
      summary: "Whole turn",
      summary_truncated: false,
      is_hidden: false,
      is_error: false,
      trajectory: {
        event_count: 2,
        tool_count: 1,
        reasoning_count: 0,
        agent_activity_count: 0,
        error_count: 0,
        unknown_count: 0,
        started_at: "2026-08-31T00:00:00Z",
        ended_at: "2026-08-31T01:00:00Z",
        duration_ms: "3600000",
      },
      tool: null,
      usage: null,
      reasoning: null,
    }],
    next_cursor: null,
    previous_cursor: null,
    total_events: 1,
    history_status: "complete",
  };
}

function trajectoryChildPage(): TrajectoryEventPageResponse {
  const child = toolEventPage().events[0]!;
  return {
    events: [{
      ...child,
      event_key: "event.v1.trajectory-tool",
      timestamp: "2026-08-31T00:10:00Z",
    }],
    next_cursor: null,
    previous_cursor: null,
    total_events: 1,
  };
}

function ViewerPageCommitProbe() {
  const state = useViewerState();
  return (
    <>
      <button
        disabled={state.sessions.length === 0}
        onClick={() => state.selectSession(state.sessions[0]!.session_key)}
        type="button"
      >
        Select indexed session
      </button>
      <output
        data-owner={state.eventsOwnerKey ?? ""}
        data-session={state.selectedSessionKey ?? ""}
        data-testid="viewer-page-commit-state"
      >
        {state.initialPageLoaded ? "ready" : "loading"}
      </output>
    </>
  );
}

describe("useViewerState session-index signalling", () => {
  it("keeps the initial catalog metadata-only until the user chooses a session", async () => {
    const indexedSession = session("codex:index-only");
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [indexedSession],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue(toolEventPage());
    const { result } = renderHook(() => useViewerState());

    await waitFor(() => expect(result.current.sessions).toEqual([indexedSession]));
    expect(result.current.selectedSessionKey).toBeNull();
    expect(result.current.selectedSession).toBeNull();
    expect(loadEventPage).not.toHaveBeenCalled();

    act(() => result.current.selectSession(indexedSession.session_key));
    await waitFor(() => {
      expect(loadEventPage).toHaveBeenCalledWith({
        session_key: indexedSession.session_key,
        direction: "backward",
        limit: 80,
      });
    });
  });

  it("subscribes to index changes before the initial catalog query", async () => {
    const subscription = deferred<() => void>();
    vi.mocked(listenForSessionIndexChanges).mockImplementation(() => subscription.promise);
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [],
      next_cursor: null,
      source_errors: [],
      pending_providers: ["codex"],
    });

    renderHook(() => useViewerState());

    expect(listSessions).not.toHaveBeenCalled();
    await act(async () => {
      subscription.resolve(vi.fn());
      await subscription.promise;
    });
    await waitFor(() => expect(listSessions).toHaveBeenCalledOnce());
  });

  it("subscribes to index progress before its snapshot and ignores an older snapshot", async () => {
    const snapshot = deferred<SessionIndexProgress>();
    let progressHandler: ((progress: SessionIndexProgress) => void) | undefined;
    vi.mocked(listenForSessionIndexProgress).mockImplementation((handler) => {
      progressHandler = handler;
      return Promise.resolve(vi.fn());
    });
    vi.mocked(getSessionIndexProgress).mockReturnValue(snapshot.promise);

    const { result } = renderHook(() => useViewerState());

    await waitFor(() => expect(listenForSessionIndexProgress).toHaveBeenCalledOnce());
    await waitFor(() => expect(getSessionIndexProgress).toHaveBeenCalledOnce());
    expect(
      vi.mocked(listenForSessionIndexProgress).mock.invocationCallOrder[0],
    ).toBeLessThan(vi.mocked(getSessionIndexProgress).mock.invocationCallOrder[0]!);

    act(() => progressHandler?.(indexProgress({ revision: "12", activity: "body", is_refreshing: true })));
    await waitFor(() => expect(result.current.sessionIndexProgress?.revision).toBe("12"));

    await act(async () => {
      snapshot.resolve(indexProgress({ revision: "11" }));
      await snapshot.promise;
    });

    expect(result.current.sessionIndexProgress?.revision).toBe("12");
    expect(result.current.sessionIndexProgress?.activity).toBe("body");
  });

  it("uses the retry command for both the status action and sidebar retry", async () => {
    const waiting = indexProgress({
      revision: "2",
      activity: "waiting_to_retry",
      body: {
        ...indexProgress().body,
        pending_jobs: 4,
      },
    });
    vi.mocked(getSessionIndexProgress).mockResolvedValue(indexProgress());
    vi.mocked(retrySessionIndex).mockResolvedValue(waiting);
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    const { result } = renderHook(() => useViewerState());

    await waitFor(() => expect(result.current.sessionIndexProgress?.revision).toBe("1"));
    await waitFor(() => expect(listSessions).toHaveBeenCalledOnce());
    await act(async () => {
      await result.current.retrySessionIndex();
    });
    expect(retrySessionIndex).toHaveBeenCalledOnce();
    expect(result.current.sessionIndexProgress?.revision).toBe("2");

    act(() => result.current.retrySessions());
    await waitFor(() => expect(retrySessionIndex).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(listSessions).toHaveBeenCalledTimes(2));
  });

  it("keeps a retry command failure alongside the last-known index snapshot", async () => {
    vi.mocked(getSessionIndexProgress).mockResolvedValue(indexProgress());
    vi.mocked(retrySessionIndex).mockRejectedValue(new Error("Session index scheduler is unavailable."));
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    const { result } = renderHook(() => useViewerState());

    await waitFor(() => expect(result.current.sessionIndexProgress?.revision).toBe("1"));
    await act(async () => {
      await result.current.retrySessionIndex();
    });

    expect(result.current.sessionIndexProgress?.revision).toBe("1");
    expect(result.current.sessionIndexProgressError).toBe("Session index scheduler is unavailable.");
  });

  it("acknowledges only after React commits an accepted initial event page", async () => {
    const indexedSession = session("codex:indexed");
    const page = deferred<EventPageResponse>();
    const pageStateAtAcknowledgement: Array<{
      eventsOwnerKey: string | null;
      initialPageLoaded: boolean;
      sessionKey: string | null;
    }> = [];
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [indexedSession],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage).mockImplementationOnce(() => page.promise);

    render(<ViewerPageCommitProbe />);

    await waitFor(() => expect(listSessions).toHaveBeenCalledOnce());
    expect(loadEventPage).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Select indexed session" }));
    await waitFor(() => expect(loadEventPage).toHaveBeenCalledOnce());
    vi.mocked(acknowledgeSessionAttention).mockImplementation(() => {
      const pageState = screen.getByTestId("viewer-page-commit-state");
      pageStateAtAcknowledgement.push({
        eventsOwnerKey: pageState.getAttribute("data-owner"),
        initialPageLoaded: pageState.textContent === "ready",
        sessionKey: pageState.getAttribute("data-session"),
      });
      return Promise.resolve({ changed: true });
    });
    await act(async () => {
      page.resolve({
        ...toolEventPage(),
        attention_revision: "7",
      });
      await Promise.resolve();
    });

    await waitFor(() => {
      expect(acknowledgeSessionAttention).toHaveBeenCalledWith({
        session_key: indexedSession.session_key,
        attention_revision: "7",
      });
    });
    expect(pageStateAtAcknowledgement).toEqual([{
      eventsOwnerKey: indexedSession.session_key,
      initialPageLoaded: true,
      sessionKey: indexedSession.session_key,
    }]);
    await waitFor(() => expect(listSessions).toHaveBeenCalledTimes(2));
  });

  it("does not acknowledge a page invalidated by a selection change before commit", async () => {
    const first = session("codex:first");
    const second = session("codex:second");
    const firstPage = deferred<EventPageResponse>();
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [first, second],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage)
      .mockImplementationOnce(() => firstPage.promise)
      .mockImplementationOnce(() => new Promise(() => undefined));
    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, first.session_key);
    await waitFor(() => expect(loadEventPage).toHaveBeenCalledOnce());
    await act(async () => {
      firstPage.resolve({
        ...toolEventPage(),
        attention_revision: "10",
      });
      // The response handler runs in this microtask, but React has not
      // committed its state updates yet. Selecting another session here is
      // the race that must suppress the acknowledgement.
      await Promise.resolve();
      result.current.selectSession(second.session_key);
    });

    await waitFor(() => expect(loadEventPage).toHaveBeenCalledTimes(2));
    expect(acknowledgeSessionAttention).not.toHaveBeenCalled();
  });

  it("does not acknowledge an event page React discarded as stale", async () => {
    const first = session("codex:first");
    const second = session("codex:second");
    const stalePage = deferred<EventPageResponse>();
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [first, second],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage)
      .mockImplementationOnce(() => stalePage.promise)
      .mockResolvedValueOnce(toolEventPage());
    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, first.session_key);
    await waitFor(() => expect(loadEventPage).toHaveBeenCalledTimes(1));
    act(() => result.current.selectSession(second.session_key));
    await waitFor(() => expect(loadEventPage).toHaveBeenCalledTimes(2));
    await act(async () => {
      stalePage.resolve({
        ...toolEventPage(),
        attention_revision: "9",
      });
      await stalePage.promise;
    });

    expect(acknowledgeSessionAttention).not.toHaveBeenCalled();
  });

  it("reloads the sidebar after an index-change event and unregisters on cleanup", async () => {
    const unlisten = vi.fn();
    let emitIndexChange: ((change: SessionIndexChangedEvent) => void) | undefined;
    vi.mocked(listenForSessionIndexChanges).mockImplementation((handler) => {
      emitIndexChange = handler;
      return Promise.resolve(unlisten);
    });
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [session("codex:listener")],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue(toolEventPage());
    const { unmount } = renderHook(() => useViewerState());

    await waitFor(() => expect(emitIndexChange).toBeDefined());
    await waitFor(() => expect(listSessions).toHaveBeenCalledTimes(1));
    expect(loadEventPage).not.toHaveBeenCalled();
    act(() => emitIndexChange?.({ changed: true, attention_session_keys: [] }));
    await waitFor(() => expect(listSessions).toHaveBeenCalledTimes(2));
    expect(loadEventPage).not.toHaveBeenCalled();
    unmount();
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("retains an explicitly selected child across a root catalog refresh", async () => {
    const root = session("codex:parent");
    root.child_count = 1;
    const child = session("codex:child");
    child.parent_session_id = root.session_id;
    child.is_subagent = true;
    let emitIndexChange: ((change: SessionIndexChangedEvent) => void) | undefined;
    vi.mocked(listenForSessionIndexChanges).mockImplementation((handler) => {
      emitIndexChange = handler;
      return Promise.resolve(vi.fn());
    });
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [root],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue(toolEventPage());
    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, root.session_key);
    await waitFor(() => expect(result.current.selectedSessionKey).toBe(root.session_key));
    act(() => result.current.openSubagent(root.session_key, child));
    await waitFor(() => expect(result.current.selectedSessionKey).toBe(child.session_key));
    await waitFor(() => expect(loadEventPage).toHaveBeenCalledTimes(2));

    act(() => emitIndexChange?.({ changed: true, attention_session_keys: [] }));
    await waitFor(() => expect(listSessions).toHaveBeenCalledTimes(2));
    expect(result.current.selectedSessionKey).toBe(child.session_key);
    expect(result.current.selectedSession?.session_key).toBe(child.session_key);
    expect(loadEventPage).toHaveBeenCalledTimes(2);
  });

  it("keeps the visible session when a background catalog refresh fails", async () => {
    const selected = session("codex:refresh-failure");
    let emitIndexChange: ((change: SessionIndexChangedEvent) => void) | undefined;
    vi.mocked(listenForSessionIndexChanges).mockImplementation((handler) => {
      emitIndexChange = handler;
      return Promise.resolve(vi.fn());
    });
    vi.mocked(listSessions)
      .mockResolvedValueOnce({
        sessions: [selected],
        next_cursor: null,
        source_errors: [],
        pending_providers: [],
      })
      .mockRejectedValueOnce(new Error("local index is temporarily unavailable"));
    vi.mocked(loadEventPage).mockResolvedValue(toolEventPage());
    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, selected.session_key);
    await waitFor(() => expect(loadEventPage).toHaveBeenCalledOnce());
    act(() => emitIndexChange?.({ changed: true, attention_session_keys: [] }));

    await waitFor(() => {
      expect(result.current.sessionsError).toBe("local index is temporarily unavailable");
    });
    expect(result.current.sessions).toEqual([selected]);
    expect(result.current.selectedSessionKey).toBe(selected.session_key);
    expect(result.current.selectedSession?.session_key).toBe(selected.session_key);
    expect(loadEventPage).toHaveBeenCalledOnce();
  });

  it("reloads and acknowledges only a selected session named by index attention", async () => {
    const selected = session("codex:selected");
    let emitIndexChange: ((change: SessionIndexChangedEvent) => void) | undefined;
    vi.mocked(listenForSessionIndexChanges).mockImplementation((handler) => {
      emitIndexChange = handler;
      return Promise.resolve(vi.fn());
    });
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [selected],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage)
      .mockResolvedValueOnce(toolEventPage())
      .mockResolvedValueOnce({ ...toolEventPage(), attention_revision: "12" });

    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, selected.session_key);
    await waitFor(() => expect(loadEventPage).toHaveBeenCalledTimes(1));
    act(() => emitIndexChange?.({
      changed: true,
      attention_session_keys: [selected.session_key],
    }));
    await waitFor(() => expect(loadEventPage).toHaveBeenCalledTimes(2));
    await waitFor(() => {
      expect(acknowledgeSessionAttention).toHaveBeenCalledWith({
        session_key: selected.session_key,
        attention_revision: "12",
      });
    });
  });
});

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
        is_subagent: false,
        provider: "codex",
        title: "Tool session",
        preview: "Run the checks",
        project: "viewer",
        cwd: "/work/repo",
        updated_at_ms: 1,
        timestamp: "2026-08-31T00:00:00Z",
        agent_path: null,
        agent_nickname: null,
        agent_role: null,
        child_count: 0,
        message_count: null,
        event_count: 1,
        history_status: "complete",
        has_unread: false,
      }],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
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
        usage: null,
        reasoning: null,
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

    await selectListedSession(result, "codex:session-1");
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
      pending_providers: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue(toolEventPage());
    vi.mocked(loadEventDetail)
      .mockImplementationOnce(() => oldA.promise)
      .mockImplementationOnce(() => freshA.promise);
    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, "codex:a");
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

  it("refreshes expanded and Inspector detail after a same-session event update", async () => {
    const stale = deferred<EventDetail>();
    const fresh = deferred<EventDetail>();
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [session("codex:session-1")],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage)
      .mockResolvedValueOnce(pendingToolEventPage())
      .mockResolvedValueOnce(toolEventPage());
    vi.mocked(loadEventDetail)
      .mockImplementationOnce(() => stale.promise)
      .mockImplementationOnce(() => fresh.promise);
    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, "codex:session-1");
    await waitFor(() => expect(result.current.events).toHaveLength(1));
    act(() => result.current.selectEvent("event.v1.1"));
    act(() => result.current.toggleEventExpanded("event.v1.1"));
    await waitFor(() => expect(loadEventDetail).toHaveBeenCalledTimes(1));

    act(() => result.current.retryEvents());
    await waitFor(() => expect(loadEventPage).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(loadEventDetail).toHaveBeenCalledTimes(2));

    await act(async () => {
      fresh.resolve(toolDetail("fresh output"));
      await fresh.promise;
    });
    await waitFor(() => {
      expect(result.current.detail?.tool_output?.sections[0]?.text).toBe("fresh output");
      expect(result.current.expandedDetail?.tool_output?.sections[0]?.text).toBe("fresh output");
    });

    await act(async () => {
      stale.resolve(toolDetail("stale output"));
      await stale.promise;
    });
    expect(result.current.detail?.tool_output?.sections[0]?.text).toBe("fresh output");
    expect(result.current.expandedDetail?.tool_output?.sections[0]?.text).toBe("fresh output");

    act(() => result.current.toggleEventExpanded("event.v1.1"));
    await waitFor(() => expect(result.current.expandedEventKey).toBeNull());
    act(() => result.current.toggleEventExpanded("event.v1.1"));
    await waitFor(() => {
      expect(result.current.expandedDetail?.tool_output?.sections[0]?.text).toBe("fresh output");
    });

    act(() => result.current.closeInspector());
    await waitFor(() => expect(result.current.inspectorOpen).toBe(false));
    act(() => result.current.toggleInspector());
    await waitFor(() => {
      expect(result.current.detail?.tool_output?.sections[0]?.text).toBe("fresh output");
    });
    expect(loadEventDetail).toHaveBeenCalledTimes(2);
  });
});

describe("useViewerState expanded reasoning detail", () => {
  it("loads readable reasoning only after expansion and shares the detail cache", async () => {
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [session("codex:reasoning")],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue(reasoningEventPage());
    vi.mocked(loadEventDetail).mockResolvedValue({
      event_key: "event.v1.reasoning",
      event: { type: "reasoning", summary: "Inspect the source" },
      native: null,
      is_hidden: false,
      tool_output: null,
    });
    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, "codex:reasoning");
    await waitFor(() => expect(result.current.events).toHaveLength(1));
    expect(loadEventDetail).not.toHaveBeenCalled();

    act(() => result.current.toggleEventExpanded("event.v1.reasoning"));
    await waitFor(() => expect(result.current.expandedDetail?.event).toMatchObject({ type: "reasoning" }));
    expect(loadEventDetail).toHaveBeenCalledOnce();

    act(() => result.current.selectEvent("event.v1.reasoning"));
    await waitFor(() => expect(result.current.detail?.event).toMatchObject({ type: "reasoning" }));
    expect(loadEventDetail).toHaveBeenCalledOnce();
  });

  it("does not request opaque, redacted, or hidden reasoning detail", async () => {
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [session("codex:opaque")],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue(reasoningEventPage({
      event_key: "event.v1.opaque",
      summary: "Reasoning redacted by provider",
      is_hidden: false,
      reasoning: {
        preview: null,
        has_summary: false,
        has_text: false,
        has_encrypted_content: true,
        is_redacted: true,
      },
    }));
    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, "codex:opaque");
    await waitFor(() => expect(result.current.events).toHaveLength(1));
    act(() => result.current.toggleEventExpanded("event.v1.opaque"));
    await waitFor(() => expect(result.current.expandedEventKey).toBe("event.v1.opaque"));
    expect(result.current.expandedDetailLoading).toBe(false);
    expect(loadEventDetail).not.toHaveBeenCalled();
  });
});

describe("useViewerState whole-turn trajectories", () => {
  it("loads a bounded child page only after opening and gives child tools their normal detail path", async () => {
    const root = session("codex:trajectory-root");
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [root],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue(trajectoryEventPage());
    vi.mocked(loadTrajectoryEventPage).mockResolvedValue(trajectoryChildPage());
    vi.mocked(loadEventDetail).mockResolvedValue({
      event_key: "event.v1.trajectory-tool",
      event: { type: "tool_call" },
      native: null,
      is_hidden: false,
      tool_output: {
        sections: [{ label: "stdout", text: "nested output", format: "text" }],
        truncated: false,
        original_size_bytes: 13,
        source_event_key: "event.v1.trajectory-tool",
      },
    });
    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, root.session_key);
    await waitFor(() => expect(result.current.events).toHaveLength(1));
    expect(loadTrajectoryEventPage).not.toHaveBeenCalled();
    expect(loadEventDetail).not.toHaveBeenCalled();

    act(() => result.current.toggleEventExpanded("trajectory.v1.turn-1"));
    await waitFor(() => {
      expect(result.current.trajectoryPages.get(root.session_key)?.get("trajectory.v1.turn-1")?.events)
        .toHaveLength(1);
    });
    expect(loadTrajectoryEventPage).toHaveBeenCalledWith({
      session_key: root.session_key,
      trajectory_key: "trajectory.v1.turn-1",
      cursor: undefined,
      direction: "forward",
      limit: 40,
    });
    expect(loadEventDetail).not.toHaveBeenCalled();

    act(() => result.current.toggleTrajectoryEventExpanded(
      "trajectory.v1.turn-1",
      "event.v1.trajectory-tool",
    ));
    await waitFor(() => {
      expect(result.current.expandedTrajectoryDetail?.tool_output?.sections[0]?.text)
        .toBe("nested output");
    });
    expect(result.current.expandedEventKey).toBe("trajectory.v1.turn-1");
    expect(result.current.expandedTrajectoryEventKey).toBe("event.v1.trajectory-tool");
    expect(loadEventDetail).toHaveBeenCalledWith({
      session_key: root.session_key,
      event_key: "event.v1.trajectory-tool",
    });

    act(() => result.current.selectEvent("event.v1.trajectory-tool"));
    await waitFor(() => {
      expect(result.current.selectedEvent?.event_key).toBe("event.v1.trajectory-tool");
    });
    expect(loadEventDetail).toHaveBeenCalledOnce();
  });

  it("ignores a stale child page after the selected session changes", async () => {
    const root = session("codex:trajectory-root");
    const nextSession = session("codex:next-session");
    const childPage = deferred<TrajectoryEventPageResponse>();
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [root, nextSession],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue(trajectoryEventPage());
    vi.mocked(loadTrajectoryEventPage).mockImplementation(() => childPage.promise);
    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, root.session_key);
    await waitFor(() => expect(result.current.selectedSessionKey).toBe(root.session_key));
    act(() => result.current.toggleEventExpanded("trajectory.v1.turn-1"));
    await waitFor(() => expect(loadTrajectoryEventPage).toHaveBeenCalledOnce());

    act(() => result.current.selectSession(nextSession.session_key));
    await waitFor(() => expect(result.current.selectedSessionKey).toBe(nextSession.session_key));

    await act(async () => {
      childPage.resolve(trajectoryChildPage());
      await childPage.promise;
    });

    expect(result.current.trajectoryPages.has(root.session_key)).toBe(false);
  });

  it("preserves cursor direction for earlier and later child pages", async () => {
    const root = session("codex:trajectory-cursors");
    const initial = {
      ...trajectoryChildPage(),
      previous_cursor: "earlier-cursor",
      next_cursor: "later-cursor",
      total_events: 3,
    };
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [root],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue(trajectoryEventPage());
    vi.mocked(loadTrajectoryEventPage).mockResolvedValue(initial);
    const { result } = renderHook(() => useViewerState());

    await selectListedSession(result, root.session_key);
    await waitFor(() => expect(result.current.events).toHaveLength(1));
    act(() => result.current.toggleEventExpanded("trajectory.v1.turn-1"));
    await waitFor(() => {
      expect(result.current.trajectoryPages.get(root.session_key)?.get("trajectory.v1.turn-1")
        ?.previous_cursor).toBe("earlier-cursor");
    });

    act(() => result.current.loadOlderTrajectoryEvents("trajectory.v1.turn-1"));
    await waitFor(() => expect(loadTrajectoryEventPage).toHaveBeenCalledTimes(2));
    expect(vi.mocked(loadTrajectoryEventPage).mock.calls[1]?.[0]).toMatchObject({
      session_key: root.session_key,
      trajectory_key: "trajectory.v1.turn-1",
      cursor: "earlier-cursor",
      direction: "backward",
      limit: 40,
    });
    await waitFor(() => {
      expect(result.current.trajectoryPages.get(root.session_key)?.get("trajectory.v1.turn-1")
        ?.is_loading_older).toBe(false);
    });

    act(() => result.current.loadNewerTrajectoryEvents("trajectory.v1.turn-1"));
    await waitFor(() => expect(loadTrajectoryEventPage).toHaveBeenCalledTimes(3));
    expect(vi.mocked(loadTrajectoryEventPage).mock.calls[2]?.[0]).toMatchObject({
      session_key: root.session_key,
      trajectory_key: "trajectory.v1.turn-1",
      cursor: "later-cursor",
      direction: "forward",
      limit: 40,
    });
  });
});

describe("useViewerState subagent discovery", () => {
  it("loads direct child metadata lazily and lets a child own the event timeline", async () => {
    const root = session("codex:root");
    root.child_count = 1;
    const child = session("codex:child");
    child.parent_session_id = root.session_id;
    child.is_subagent = true;
    child.agent_nickname = "Hubble";
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [root],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(listSessionChildren).mockResolvedValue({
      sessions: [child],
      next_cursor: null,
    });

    const { result } = renderHook(() => useViewerState());
    await selectListedSession(result, root.session_key);
    await waitFor(() => expect(result.current.selectedSession?.session_key).toBe(root.session_key));
    expect(listSessionChildren).not.toHaveBeenCalled();

    act(() => result.current.loadSessionChildren(root.session_key));
    await waitFor(() => {
      expect(result.current.sessionChildren.get(root.session_key)?.sessions).toEqual([child]);
    });
    expect(listSessionChildren).toHaveBeenCalledWith({
      parent_session_key: root.session_key,
      cursor: undefined,
      limit: 60,
    });

    act(() => result.current.selectSession(child.session_key));
    await waitFor(() => expect(result.current.selectedSession?.session_key).toBe(child.session_key));
  });

  it("opens a delegation child before its lazy sidebar page arrives", async () => {
    const root = session("codex:delegating-root");
    root.child_count = 1;
    const child = session("codex:delegated-child");
    child.parent_session_id = root.session_id;
    child.is_subagent = true;
    child.agent_nickname = "Hubble";
    const childPage = deferred<{ sessions: SessionSummary[]; next_cursor: string | null }>();
    vi.mocked(listSessions).mockResolvedValue({
      sessions: [root],
      next_cursor: null,
      source_errors: [],
      pending_providers: [],
    });
    vi.mocked(loadEventPage).mockResolvedValue(toolEventPage());
    vi.mocked(listSessionChildren).mockImplementation(() => childPage.promise);

    const { result } = renderHook(() => useViewerState());
    await selectListedSession(result, root.session_key);
    await waitFor(() => expect(result.current.selectedSession?.session_key).toBe(root.session_key));

    act(() => result.current.openSubagent(root.session_key, child));

    await waitFor(() => {
      expect(result.current.selectedSession?.session_key).toBe(child.session_key);
      expect(result.current.sessionChildren.get(root.session_key)?.sessions).toEqual([child]);
    });
    expect(listSessionChildren).toHaveBeenCalledWith({
      parent_session_key: root.session_key,
      cursor: undefined,
      limit: 60,
    });

    await act(async () => {
      childPage.resolve({ sessions: [], next_cursor: null });
      await childPage.promise;
    });

    await waitFor(() => {
      expect(result.current.sessionChildren.get(root.session_key)?.sessions).toEqual([child]);
    });
  });
});
