import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { SessionDrawer } from "./SessionDrawer";

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it("focuses mobile search, dismisses on Escape, and preserves sidebar state across resizing", () => {
  let matches = true;
  let resize = () => {};
  vi.stubGlobal("matchMedia", () => ({
    get matches() { return matches; },
    addEventListener: (_: string, listener: () => void) => { resize = listener; },
    removeEventListener: vi.fn(),
  }));
  const close = vi.fn();
  const view = (is_open: boolean) => <SessionDrawer is_open={is_open} on_close={close}>
    <input aria-label="Search sessions" type="search" defaultValue="saved search" />
  </SessionDrawer>;
  const { rerender } = render(view(false));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  rerender(view(true));
  expect(screen.getByRole("searchbox")).toHaveFocus();
  fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
  expect(close).toHaveBeenCalledOnce();
  rerender(view(false));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  act(() => { matches = false; resize(); });
  expect(screen.getByRole("searchbox")).toHaveValue("saved search");
  expect(screen.getByRole("dialog")).toHaveAttribute("open");
  act(() => { matches = true; resize(); });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
