import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SessionIndexProgress, SourceError } from "../lib/types";
import { StatusBar } from "./StatusBar";

afterEach(cleanup);

function progress(overrides: Partial<SessionIndexProgress> = {}): SessionIndexProgress {
  return {
    revision: "9",
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
      completed_in_run: 3,
      stale_in_run: 0,
      batch_size: 1,
      providers: [],
    },
    worker_error: null,
    retry_at_ms: null,
    ...overrides,
  };
}

function renderStatusBar(
  progressValue: SessionIndexProgress | null = progress(),
  sourceErrors: SourceError[] = [],
  onRetry = vi.fn(),
  error: string | null = null,
) {
  render(
    <>
      <button type="button">Outside</button>
      <StatusBar
        error={error}
        is_loading={false}
        is_retrying={false}
        on_retry={onRetry}
        progress={progressValue}
        source_errors={sourceErrors}
      />
    </>,
  );
  return onRetry;
}

describe("StatusBar", () => {
  it("treats a pending initial catalog as active and shows every provider state", () => {
    renderStatusBar(progress({
      catalog: {
        active_provider: "codex",
        processed_providers: 1,
        total_providers: 6,
        pending_providers: ["codex", "pi"],
        error_providers: ["dsh"],
      },
      body: {
        ...progress().body,
        pending_jobs: 3,
        providers: [{ provider: "opencode", pending_jobs: 3, failed_jobs: 0 }],
      },
    }), [{ provider: "dsh", message: "Session log is unavailable." }]);

    expect(screen.getByText("Indexing catalog · 1 of 6 providers")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /open notifications/i }));

    expect(screen.getByRole("dialog", { name: "Notifications" })).toHaveFocus();
    expect(screen.getByLabelText("Cataloging: Reading session headers.")).toBeInTheDocument();
    expect(screen.getByLabelText("Backfilling: 3 session details pending.")).toBeInTheDocument();
    expect(screen.getByLabelText("Needs attention: Session log is unavailable.")).toBeInTheDocument();
    expect(screen.getAllByLabelText("Ready: Catalog is current.")).toHaveLength(2);
  });

  it("closes on Escape and outside click while restoring focus to its bell", () => {
    renderStatusBar();
    const bell = screen.getByRole("button", { name: /open notifications/i });
    fireEvent.click(bell);
    expect(screen.getByRole("dialog", { name: "Notifications" })).toBeInTheDocument();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Notifications" })).not.toBeInTheDocument();
    expect(bell).toHaveFocus();

    fireEvent.click(bell);
    fireEvent.pointerDown(screen.getByRole("button", { name: "Outside" }));
    expect(screen.queryByRole("dialog", { name: "Notifications" })).not.toBeInTheDocument();
    expect(bell).toHaveFocus();
  });

  it("offers the real retry action without a notification history", () => {
    const onRetry = renderStatusBar(progress({
      activity: "waiting_to_retry",
      body: {
        ...progress().body,
        pending_jobs: 2,
      },
    }));

    expect(screen.getByText("Indexing session details · 2 remaining")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /open notifications/i }));
    fireEvent.click(screen.getByRole("button", { name: "Retry indexing" }));

    expect(onRetry).toHaveBeenCalledOnce();
    expect(screen.queryByText("Previous notifications")).not.toBeInTheDocument();
  });

  it("shows a generic worker failure without exposing scheduler error text", () => {
    renderStatusBar(progress({
      activity: "waiting_to_retry",
      worker_error: "task_failed",
    }));

    expect(screen.getByText("Session index worker needs attention · retry scheduled")).toBeInTheDocument();
    expect(document.querySelector(".status-bar__spinner")).not.toBeInTheDocument();
    const bell = screen.getByRole("button", { name: /open notifications/i });
    expect(bell).toHaveAttribute("data-has-attention", "true");
    fireEvent.click(bell);

    expect(screen.getByText("Session index worker")).toBeInTheDocument();
    expect(screen.getByText(
      "The background session index task stopped unexpectedly. Retry indexing to start another pass.",
    )).toBeInTheDocument();
    expect(screen.queryByText(/stack trace|\/Users\//i)).not.toBeInTheDocument();
  });

  it("keeps a failed retry visible even when a last-known index snapshot exists", () => {
    renderStatusBar(progress(), [], vi.fn(), "Session index scheduler is unavailable.");

    expect(screen.getByText("Session index needs attention")).toBeInTheDocument();
    expect(screen.queryByText("Session index is up to date")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /open notifications/i }));
    expect(screen.getByText("Session index control")).toBeInTheDocument();
    expect(screen.getAllByText("Session index scheduler is unavailable.")).toHaveLength(2);
  });

  it("keeps changing counts out of its polite phase announcement", () => {
    const first = progress({
      activity: "body",
      is_refreshing: true,
      body: {
        ...progress().body,
        pending_jobs: 4,
      },
    });
    const { container, rerender } = render(
      <StatusBar
        error={null}
        is_loading={false}
        is_retrying={false}
        on_retry={vi.fn()}
        progress={first}
        source_errors={[]}
      />,
    );

    expect(container.querySelector(".status-bar__summary")).not.toHaveAttribute("aria-live");
    expect(screen.getByRole("status")).toHaveTextContent("Session detail indexing started.");
    expect(screen.getByRole("status")).not.toHaveTextContent("4");

    rerender(
      <StatusBar
        error={null}
        is_loading={false}
        is_retrying={false}
        on_retry={vi.fn()}
        progress={{
          ...first,
          revision: "10",
          body: { ...first.body, pending_jobs: 3 },
        }}
        source_errors={[]}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Session detail indexing started.");
    expect(screen.getByRole("status")).not.toHaveTextContent("3");

    rerender(
      <StatusBar
        error={null}
        is_loading={false}
        is_retrying={false}
        on_retry={vi.fn()}
        progress={{
          ...first,
          revision: "11",
          body: { ...first.body, failed_jobs: 1 },
        }}
        source_errors={[]}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Session index needs attention.");
  });
});
