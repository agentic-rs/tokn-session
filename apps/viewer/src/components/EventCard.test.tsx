import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { EventDetail, EventSummary, ToolCardSummary } from "../lib/types";
import { EventCard } from "./EventCard";

afterEach(cleanup);

function tool(overrides: Partial<ToolCardSummary> = {}): ToolCardSummary {
  return {
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
    ...overrides,
  };
}

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
    tool: null,
    ...overrides,
  };
}

function detail(overrides: Partial<EventDetail> = {}): EventDetail {
  return {
    event_key: "event.v1.1",
    event: { type: "tool_call" },
    native: null,
    is_hidden: false,
    tool_output: null,
    ...overrides,
  };
}

function renderCard(
  cardEvent: EventSummary,
  overrides: Partial<React.ComponentProps<typeof EventCard>> = {},
) {
  return render(
    <EventCard
      button_id="event-button"
      detail={null}
      detail_error={null}
      detail_loading={false}
      event={cardEvent}
      is_expanded={false}
      is_selected={false}
      on_retry_detail={vi.fn()}
      on_select={vi.fn()}
      on_toggle={vi.fn()}
      {...overrides}
    />,
  );
}

describe("EventCard conversation content", () => {
  it("renders visible conversation Markdown outside the inspect button", () => {
    const onSelect = vi.fn();
    const { container } = renderCard(event(), { on_select: onSelect });

    expect(screen.getByText("formatted").tagName).toBe("STRONG");
    expect(container.querySelector("button strong")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Inspect assistant message" }));
    expect(onSelect).toHaveBeenCalledWith("event.v1.1");
  });

  it("keeps hidden messages redacted and exposes the full-message action for truncation", () => {
    const { rerender } = renderCard(event({ is_hidden: true, summary: "secret" }));

    expect(screen.getByText("Hidden extension message")).toBeInTheDocument();
    expect(screen.queryByText("secret")).not.toBeInTheDocument();

    rerender(
      <EventCard
        button_id="event-button"
        detail={null}
        detail_error={null}
        detail_loading={false}
        event={event({ summary_truncated: true })}
        is_expanded={false}
        is_selected={false}
        on_retry_detail={vi.fn()}
        on_select={vi.fn()}
        on_toggle={vi.fn()}
      />,
    );
    expect(screen.getByText("View full message")).toBeInTheDocument();
  });

  it("keeps reasoning Markdown local and unknown events expanded by default", () => {
    const { unmount } = renderCard(event({
      type: "reasoning",
      role: null,
      title: "Reasoning",
      summary: "## Approach\n\n- inspect the source\n- verify the **result**",
    }));

    expect(screen.queryByRole("heading", { name: "Approach" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reasoning" }));
    expect(screen.getByRole("heading", { name: "Approach" })).toBeInTheDocument();
    expect(screen.getByText("result").tagName).toBe("STRONG");
    unmount();

    renderCard(event({ type: "unknown", role: null, title: "Mystery", summary: "raw shape" }));
    expect(screen.getByRole("button", { name: "Mystery" })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("raw shape")).toBeInTheDocument();
  });
});

describe("EventCard tool headings", () => {
  it("uses compact provider-neutral headings for every known tool family", () => {
    const tools: Array<[string, Partial<ToolCardSummary>, string]> = [
      ["shell", {}, "cargo test"],
      ["file_read", { path: "src/main.rs", command: null, cwd: null, exit_code: null }, "src/main.rs"],
      ["file_write", { path: "out.txt", command: null, cwd: null, exit_code: null, bytes: 1536 }, "out.txt"],
      ["file_edit", { path: "lib.rs", command: null, cwd: null, exit_code: null, added: 8, removed: 3 }, "lib.rs"],
      ["search", { query: "ToolCall", command: null, cwd: null, exit_code: null }, "ToolCall"],
      ["web", { url: "https://example.test/docs", command: null, cwd: null, exit_code: null }, "https://example.test/docs"],
      ["task", { task_title: "Run smoke test", command: null, cwd: null, exit_code: null }, "Run smoke test"],
      ["unknown", { tool_name: "mcp_custom_lookup", command: null, cwd: null, exit_code: null }, "Mcp Custom Lookup"],
    ];

    render(
      <>
        {tools.map(([kind, overrides], index) => (
          <EventCard
            button_id={`event-button-${index}`}
            detail={null}
            detail_error={null}
            detail_loading={false}
            event={event({
              event_key: `event-${index}`,
              type: "tool_call",
              role: null,
              title: "raw_tool_name",
              summary: "raw summary",
              tool: tool({ kind, ...overrides }),
            })}
            is_expanded={false}
            is_selected={false}
            key={kind}
            on_retry_detail={vi.fn()}
            on_select={vi.fn()}
            on_toggle={vi.fn()}
          />
        ))}
      </>,
    );

    for (const [, , primary] of tools) {
      expect(screen.getByText(primary)).toBeInTheDocument();
    }
    expect(screen.getByText("/work/repo")).toBeInTheDocument();
    expect(screen.getByText("1.5 KB")).toBeInTheDocument();
    expect(screen.getByText("+8 −3")).toBeInTheDocument();
  });

  it("reports provider errors as failed even when the exit code is zero", () => {
    renderCard(event({
      type: "tool_call",
      role: null,
      title: "exec_command",
      is_error: true,
      tool: tool({ exit_code: 0 }),
    }));

    expect(screen.getByText("failed")).toHaveAttribute("data-tone", "error");
    expect(screen.queryByText("exit 0")).not.toBeInTheDocument();
  });
});

describe("EventCard tool output", () => {
  const toolEvent = event({
    type: "tool_call",
    role: null,
    title: "exec_command",
    summary: "shell exit 0 cargo test",
    tool: tool(),
  });

  it("expands independently from Inspector and exposes an explicitly labelled region", () => {
    const onSelect = vi.fn();
    const onToggle = vi.fn();
    const { rerender } = renderCard(toolEvent, { on_select: onSelect, on_toggle: onToggle });
    const toggle = screen.getByRole("button", { name: "Shell: cargo test" });

    expect(toggle).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(toggle);
    expect(onToggle).toHaveBeenCalledWith("event.v1.1");
    expect(onSelect).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Inspect cargo test" }));
    expect(onSelect).toHaveBeenCalledWith("event.v1.1");

    rerender(
      <EventCard
        button_id="event-button"
        detail={detail()}
        detail_error={null}
        detail_loading={false}
        event={toolEvent}
        is_expanded
        is_selected={false}
        on_retry_detail={vi.fn()}
        on_select={onSelect}
        on_toggle={onToggle}
      />,
    );
    const region = screen.getByRole("region", { name: "cargo test" });
    expect(toggle.id).toBe("");
    expect(screen.getByRole("button", { name: "Shell: cargo test" })).toHaveAttribute(
      "aria-controls",
      region.id,
    );
    expect(region).toHaveAttribute("aria-labelledby", "event-button-label");
  });

  it("renders output as literal selectable preformatted text, never Markdown or HTML", () => {
    const unsafe = '<img src=x onerror="globalThis.pwned=true">\n**not bold**';
    const { container } = renderCard(toolEvent, {
      detail: detail({
        tool_output: {
          sections: [
            { label: "stdout", text: unsafe, format: "text" },
            { label: "metadata", text: '{"ok":true}', format: "json" },
          ],
          truncated: true,
          original_size_bytes: 70_000,
          source_event_key: "event.v1.2",
        },
      }),
      is_expanded: true,
    });

    const blocks = container.querySelectorAll(".tool-output pre");
    expect(blocks).toHaveLength(2);
    expect(blocks[0]?.textContent).toBe(unsafe);
    expect(blocks[0]).toHaveAttribute("tabindex", "0");
    expect(blocks[0]).toHaveAccessibleName("cargo test stdout output");
    expect(blocks[1]).toHaveAttribute("data-format", "json");
    expect(blocks[1]).toHaveAccessibleName("cargo test metadata output");
    expect(container.querySelector(".tool-output img")).not.toBeInTheDocument();
    expect(container.querySelector(".tool-output strong")).not.toBeInTheDocument();
    expect(screen.getByText(/truncated from 70 KB/i)).toBeInTheDocument();
    expect(screen.getByText(/related result event/i)).toBeInTheDocument();
  });

  it("shows loading, retryable error, pending, and finished empty-output states", () => {
    const onRetry = vi.fn();
    const { rerender } = renderCard(toolEvent, {
      detail_loading: true,
      is_expanded: true,
      on_retry_detail: onRetry,
    });
    expect(screen.getByRole("status")).toHaveTextContent("Loading tool output");

    rerender(
      <EventCard
        button_id="event-button"
        detail={null}
        detail_error="bridge disconnected"
        detail_loading={false}
        event={toolEvent}
        is_expanded
        is_selected={false}
        on_retry_detail={onRetry}
        on_select={vi.fn()}
        on_toggle={vi.fn()}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("bridge disconnected");
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalledOnce();

    rerender(
      <EventCard
        button_id="event-button"
        detail={detail()}
        detail_error={null}
        detail_loading={false}
        event={{ ...toolEvent, phase: "started" }}
        is_expanded
        is_selected={false}
        on_retry_detail={onRetry}
        on_select={vi.fn()}
        on_toggle={vi.fn()}
      />,
    );
    expect(screen.getByText("Output is not available yet.")).toBeInTheDocument();

    rerender(
      <EventCard
        button_id="event-button"
        detail={detail()}
        detail_error={null}
        detail_loading={false}
        event={toolEvent}
        is_expanded
        is_selected={false}
        on_retry_detail={onRetry}
        on_select={vi.fn()}
        on_toggle={vi.fn()}
      />,
    );
    expect(screen.getByText("No output was captured for this tool call.")).toBeInTheDocument();
  });

  it("does not reveal provider-hidden tool output", () => {
    renderCard({ ...toolEvent, is_hidden: true }, {
      detail: detail({
        tool_output: {
          sections: [{ label: null, text: "secret output", format: "text" }],
          truncated: false,
          original_size_bytes: 13,
          source_event_key: "event.v1.1",
        },
      }),
      is_expanded: true,
    });

    expect(screen.getByText("Tool output is hidden by the provider.")).toBeInTheDocument();
    expect(screen.queryByText("secret output")).not.toBeInTheDocument();
  });
});
