import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getTranslationStatus } from "../lib/tauri";
import { isDesktop } from "../lib/transport";
import type { TranslationStatus } from "../lib/types";
import { TranslationProvider, useTranslationStatus } from "./TranslationProvider";

vi.mock("../lib/tauri", () => ({ getTranslationStatus: vi.fn() }));
vi.mock("../lib/transport", () => ({ isDesktop: vi.fn() }));

function Status({ label = "status" }: { label?: string }) {
  const status = useTranslationStatus();
  return <output aria-label={label}>{status === null ? "Unchecked" : status.available ? "Available" : status.reason}</output>;
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(isDesktop).mockReturnValue(true);
  vi.mocked(getTranslationStatus).mockResolvedValue({ available: true, reason: null });
});
afterEach(cleanup);

describe("TranslationProvider", () => {
  it("shares one desktop availability check across consumers and rerenders", async () => {
    const view = render(<TranslationProvider><Status label="first" /><Status label="second" /></TranslationProvider>);
    expect(await screen.findAllByText("Available")).toHaveLength(2);
    expect(getTranslationStatus).toHaveBeenCalledTimes(1);
    view.rerender(<TranslationProvider><Status label="replacement" /></TranslationProvider>);
    expect(screen.getByLabelText("replacement")).toHaveTextContent("Available");
    expect(getTranslationStatus).toHaveBeenCalledTimes(1);
  });

  it("never invokes native translation availability in the browser", async () => {
    vi.mocked(isDesktop).mockReturnValue(false);
    render(<TranslationProvider><Status /></TranslationProvider>);
    await act(async () => {});
    expect(screen.getByLabelText("status")).toHaveTextContent("Unchecked");
    expect(getTranslationStatus).not.toHaveBeenCalled();
  });

  it("provides the native reason when translation is unavailable", async () => {
    vi.mocked(getTranslationStatus).mockResolvedValue({ available: false, reason: "Requires macOS 15 or later." });
    render(<TranslationProvider><Status /></TranslationProvider>);
    expect(await screen.findByText("Requires macOS 15 or later.")).toBeInTheDocument();
  });

  it("converts availability-check failures into a readable unavailable state", async () => {
    vi.mocked(getTranslationStatus).mockRejectedValue(new Error("Missing native command"));
    render(<TranslationProvider><Status /></TranslationProvider>);
    expect(await screen.findByText("Apple Translation is unavailable in this app.")).toBeInTheDocument();
  });

  it("ignores a late availability result from an unmounted provider", async () => {
    let resolve!: (status: TranslationStatus) => void;
    vi.mocked(getTranslationStatus).mockReturnValueOnce(new Promise((done) => { resolve = done; }));
    const old = render(<TranslationProvider><Status /></TranslationProvider>);
    expect(screen.getByLabelText("status")).toHaveTextContent("Unchecked");
    old.unmount();
    vi.mocked(getTranslationStatus).mockResolvedValue({ available: false, reason: "Unavailable in this viewer." });
    render(<TranslationProvider><Status /></TranslationProvider>);
    await screen.findByText("Unavailable in this viewer.");
    await act(async () => resolve({ available: true, reason: null }));
    expect(screen.getByLabelText("status")).toHaveTextContent("Unavailable in this viewer.");
  });
});
