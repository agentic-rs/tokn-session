import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { eventButtonId } from "./state";
import { useViewerState } from "./useViewerState";

vi.mock("./tauri", () => ({
  listSessions: vi.fn(() => new Promise(() => undefined)),
  loadEventDetail: vi.fn(() => new Promise(() => undefined)),
  loadEventPage: vi.fn(() => new Promise(() => undefined)),
}));

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
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
