import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { EventSummary } from "../lib/types";
import { EventCard } from "./EventCard";

afterEach(cleanup);

function event(overrides: Partial<EventSummary> = {}): EventSummary {
  return {
    event_key: "event.v1.1",
    type: "message",
    provider: "codex",
    timestamp: "2026-08-31T00:00:00Z",
    phase: "finished",
    role: "assistant",
    title: "Assistant message",
    summary: "A **formatted** answer",
    summary_truncated: false,
    is_hidden: false,
    is_error: false,
    ...overrides,
  };
}

describe("EventCard Markdown routing", () => {
  it("renders visible conversation Markdown outside the inspect button", () => {
    const onSelect = vi.fn();
    const { container } = render(
      <EventCard
        button_id="event-button"
        event={event()}
        is_selected={false}
        on_select={onSelect}
      />,
    );

    expect(screen.getByText("formatted").tagName).toBe("STRONG");
    expect(container.querySelector("button strong")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Inspect assistant message" }));
    expect(onSelect).toHaveBeenCalledWith("event.v1.1");
  });

  it("keeps hidden messages redacted and exposes the full-message action for truncation", () => {
    const { rerender } = render(
      <EventCard
        button_id="event-button"
        event={event({ is_hidden: true, summary: "secret" })}
        is_selected={false}
        on_select={vi.fn()}
      />,
    );

    expect(screen.getByText("Hidden extension message")).toBeInTheDocument();
    expect(screen.queryByText("secret")).not.toBeInTheDocument();

    rerender(
      <EventCard
        button_id="event-button"
        event={event({ summary_truncated: true })}
        is_selected={false}
        on_select={vi.fn()}
      />,
    );
    expect(screen.getByText("View full message")).toBeInTheDocument();
  });

  it("renders reasoning Markdown only after the technical card expands", () => {
    render(
      <EventCard
        button_id="event-button"
        event={event({
          type: "reasoning",
          role: null,
          title: "Reasoning",
          summary: "## Approach\n\n- inspect the source\n- verify the **result**",
        })}
        is_selected={false}
        on_select={vi.fn()}
      />,
    );

    expect(screen.queryByRole("heading", { name: "Approach" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Reasoning/i }));
    expect(screen.getByRole("heading", { name: "Approach" })).toBeInTheDocument();
    expect(screen.getByRole("list")).toHaveTextContent("inspect the source");
    expect(screen.getByText("result").tagName).toBe("STRONG");
  });
});
