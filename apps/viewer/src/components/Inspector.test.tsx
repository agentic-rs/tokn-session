import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { EventDetail, EventSummary } from "../lib/types";
import { Inspector } from "./Inspector";

afterEach(cleanup);

const EVENT: EventSummary = {
  event_key: "event.v1.2",
  type: "message",
  provider: "codex",
  timestamp: "2026-08-31T00:00:00Z",
  phase: "finished",
  role: "assistant",
  title: "Assistant message",
  summary: "preview",
  summary_truncated: true,
  is_hidden: false,
  is_error: false,
};

const DETAIL: EventDetail = {
  event_key: EVENT.event_key,
  event: {
    type: "message",
    provider: "codex",
    role: "assistant",
    text: "# Full result\n\nThe **complete** message.",
  },
  native: null,
  is_hidden: false,
};

describe("Inspector Markdown routing", () => {
  it("renders readable message detail as Markdown in the Content tab", () => {
    render(
      <Inspector
        detail={DETAIL}
        error={null}
        event={EVENT}
        is_loading={false}
        is_open
        on_close={vi.fn()}
        on_retry={vi.fn()}
      />,
    );

    expect(screen.getByRole("tab", { name: "Content" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("heading", { name: "Full result" })).toBeInTheDocument();
    expect(screen.getByText("complete").tagName).toBe("STRONG");
  });

  it("does not expose hidden detail as readable Markdown", () => {
    render(
      <Inspector
        detail={{
          event_key: EVENT.event_key,
          event: { type: "message", provider: "pi", redacted: true },
          native: null,
          is_hidden: true,
        }}
        error={null}
        event={{ ...EVENT, provider: "pi", is_hidden: true }}
        is_loading={false}
        is_open
        on_close={vi.fn()}
        on_retry={vi.fn()}
      />,
    );

    expect(screen.getByText("Content hidden by provider")).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Content" })).not.toBeInTheDocument();
  });
});
