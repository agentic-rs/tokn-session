import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  EventDetail,
  EventSummary,
  ReasoningCardSummary,
  ToolCardSummary,
  UsageCardSummary,
} from "../lib/types";
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

function usage(overrides: Partial<UsageCardSummary> = {}): UsageCardSummary {
  return {
    kind: "model_call",
    input_tokens: "33",
    output_tokens: "5",
    total_tokens: "38",
    cache_read_tokens: "20",
    cache_write_tokens: "3",
    reasoning_tokens: "2",
    turn_id: null,
    step_id: null,
    ...overrides,
  };
}

function reasoning(overrides: Partial<ReasoningCardSummary> = {}): ReasoningCardSummary {
  return {
    preview: "Inspect the source",
    has_summary: true,
    has_text: false,
    has_encrypted_content: false,
    is_redacted: false,
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
    usage: null,
    reasoning: null,
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

  it("keeps unknown events expanded by default", () => {
    const { unmount } = renderCard(event({
      type: "unknown",
      role: null,
      title: "Mystery",
      summary: "raw shape",
    }));

    expect(screen.getByRole("button", { name: "Mystery" })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("raw shape")).toBeInTheDocument();
    unmount();
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
    expect(screen.getByText(/Inspect the event for the complete bounded detail/i)).toBeInTheDocument();
    expect(screen.queryByText(/related result event/i)).not.toBeInTheDocument();
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

describe("EventCard semantic tool operations", () => {
  it("renders Code Execution and Terminal operation headings", () => {
    render(
      <>
        <EventCard
          button_id="code-button"
          detail={null}
          detail_error={null}
          detail_loading={false}
          event={event({
            event_key: "event.code",
            type: "tool_call",
            role: null,
            title: "exec",
            tool: tool({
              kind: "code_execution",
              tool_name: "exec",
              provider_tool_name: "exec",
              language: "javascript",
              command: null,
              cwd: null,
              exit_code: null,
            }),
          })}
          is_expanded={false}
          is_selected={false}
          on_retry_detail={vi.fn()}
          on_select={vi.fn()}
          on_toggle={vi.fn()}
        />
        <EventCard
          button_id="terminal-button"
          detail={null}
          detail_error={null}
          detail_loading={false}
          event={event({
            event_key: "event.terminal",
            type: "tool_call",
            role: null,
            title: "write_stdin",
            tool: tool({
              kind: "terminal",
              tool_name: "write_stdin",
              command: null,
              cwd: null,
              exit_code: null,
              terminal_action: "wait",
              terminal_session_id: "90855",
              wait_ms: 30_000,
              status: "completed",
            }),
          })}
          is_expanded={false}
          is_selected={false}
          on_retry_detail={vi.fn()}
          on_select={vi.fn()}
          on_toggle={vi.fn()}
        />
      </>,
    );

    expect(screen.getByRole("button", { name: "Code: Javascript code" })).toBeInTheDocument();
    expect(screen.getByText("exec")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Terminal: Wait for terminal 90855" }))
      .toBeInTheDocument();
    expect(screen.getByText("Up to 30 s")).toBeInTheDocument();
  });

  it("uses the derived operation status instead of the source record phase", () => {
    const { rerender } = renderCard(event({
      type: "tool_call",
      role: null,
      title: "write_stdin",
      phase: "finished",
      tool: tool({
        kind: "terminal",
        tool_name: "write_stdin",
        command: null,
        cwd: null,
        exit_code: null,
        status: "running",
        terminal_action: "send",
        terminal_session_id: "90855",
        chars_len: 14,
      }),
    }), {
      detail: detail(),
      is_expanded: true,
    });

    expect(screen.getByRole("button", {
      name: "Terminal: Send 14 characters to terminal 90855",
    })).toBeInTheDocument();
    expect(screen.getByText("running")).toHaveAttribute("data-tone", "neutral");
    expect(screen.getByText("Output is not available yet.")).toBeInTheDocument();

    rerender(
      <EventCard
        button_id="event-button"
        detail={detail()}
        detail_error={null}
        detail_loading={false}
        event={event({
          type: "tool_call",
          role: null,
          title: "write_stdin",
          phase: "started",
          tool: tool({
            kind: "terminal",
            tool_name: "write_stdin",
            command: null,
            cwd: null,
            exit_code: null,
            status: "completed",
            terminal_action: "send",
            terminal_session_id: "90855",
            chars_len: 14,
          }),
        })}
        is_expanded
        is_selected={false}
        on_retry_detail={vi.fn()}
        on_select={vi.fn()}
        on_toggle={vi.fn()}
      />,
    );

    expect(screen.queryByText("running")).not.toBeInTheDocument();
    expect(screen.getByText("No output was captured for this tool call.")).toBeInTheDocument();
  });

  it("labels a failed logical operation as failed without relying on an exit code", () => {
    renderCard(event({
      type: "tool_call",
      role: null,
      title: "exec",
      tool: tool({
        kind: "code_execution",
        tool_name: "exec",
        command: null,
        cwd: null,
        exit_code: null,
        status: "failed",
      }),
    }));

    expect(screen.getByText("failed")).toHaveAttribute("data-tone", "error");
  });
});

describe("EventCard usage", () => {
  const usageEvent = event({
    type: "usage",
    role: null,
    title: "Usage",
    summary: "[usage] input=33 output=5 total=38",
    usage: usage(),
  });

  it("shows scope-aware, exact token data without treating subsets as totals", () => {
    renderCard(usageEvent);

    expect(screen.getByRole("button", { name: "Usage: 38 tokens" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.getByText("Model call · 33 input · 5 output")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Usage: 38 tokens" }));
    expect(screen.getByRole("region", { name: "38 tokens" })).toBeInTheDocument();
    expect(screen.getByText("Reported total")).toBeInTheDocument();
    expect(screen.getAllByText("38")).toHaveLength(1);
    expect(screen.getByText(/Cache counts are already included in input/i)).toBeInTheDocument();
  });

  it("retains zero values, exact large counters, and the snapshot warning", () => {
    renderCard({
      ...usageEvent,
      usage: usage({
        kind: "session_snapshot",
        input_tokens: "18446744073709551615",
        output_tokens: "0",
        total_tokens: null,
        cache_read_tokens: "0",
        cache_write_tokens: null,
        reasoning_tokens: "0",
      }),
    });

    const button = screen.getByRole("button", {
      name: "Usage: 18,446,744,073,709,551,615 input · 0 output",
    });
    fireEvent.click(button);
    expect(screen.getByText("18,446,744,073,709,551,615")).toBeInTheDocument();
    expect(screen.getAllByText("0")).toHaveLength(3);
    expect(screen.queryByText("Reported total")).not.toBeInTheDocument();
    expect(screen.getByText(/replaces earlier snapshots/i)).toBeInTheDocument();
  });
});

describe("EventCard reasoning", () => {
  const summary = "## Approach\n\n- inspect the source\n- verify the **result**";
  const detailed = "I checked the implementation details.";
  const reasoningEvent = event({
    type: "reasoning",
    role: null,
    title: "Reasoning",
    summary,
    reasoning: reasoning({ preview: "Approach", has_text: true }),
  });

  it("loads reasoning through the controlled detail path and keeps Inspect independent", () => {
    const onToggle = vi.fn();
    const onSelect = vi.fn();
    const { rerender } = renderCard(reasoningEvent, { on_select: onSelect, on_toggle: onToggle });
    const toggle = screen.getByRole("button", { name: "Reasoning: Approach" });

    fireEvent.click(toggle);
    expect(onToggle).toHaveBeenCalledWith("event.v1.1");
    expect(onSelect).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Inspect Approach" }));
    expect(onSelect).toHaveBeenCalledWith("event.v1.1");

    rerender(
      <EventCard
        button_id="event-button"
        detail={detail({
          event: { type: "reasoning", summary, text: detailed },
        })}
        detail_error={null}
        detail_loading={false}
        event={reasoningEvent}
        is_expanded
        is_selected={false}
        on_retry_detail={vi.fn()}
        on_select={onSelect}
        on_toggle={onToggle}
      />,
    );

    expect(screen.getByRole("heading", { name: "Approach" })).toBeInTheDocument();
    expect(screen.getByText("result").tagName).toBe("STRONG");
    expect(screen.getByText(detailed).closest("details")).not.toHaveAttribute("open");
    fireEvent.click(screen.getByText("Detailed reasoning"));
    expect(screen.getByText(detailed).closest("details")).toHaveAttribute("open");
  });

  it("shows deliberate opaque and redacted reasoning states without requesting content", () => {
    const { rerender } = renderCard({
      ...reasoningEvent,
      reasoning: reasoning({
        preview: null,
        has_summary: false,
        has_text: false,
        has_encrypted_content: true,
      }),
    }, { is_expanded: true });
    expect(screen.getByText(/encrypted and cannot be shown inline/i)).toBeInTheDocument();
    expect(screen.queryByText(summary)).not.toBeInTheDocument();

    rerender(
      <EventCard
        button_id="event-button"
        detail={null}
        detail_error={null}
        detail_loading={false}
        event={{
          ...reasoningEvent,
          reasoning: reasoning({
            preview: null,
            has_summary: false,
            has_text: false,
            is_redacted: true,
          }),
        }}
        is_expanded
        is_selected={false}
        on_retry_detail={vi.fn()}
        on_select={vi.fn()}
        on_toggle={vi.fn()}
      />,
    );
    expect(screen.getByText(/redacted by the provider/i)).toBeInTheDocument();
  });

  it("shows loading and a retryable detail failure", () => {
    const onRetry = vi.fn();
    const { rerender } = renderCard(reasoningEvent, {
      detail_loading: true,
      is_expanded: true,
      on_retry_detail: onRetry,
    });
    expect(screen.getByRole("status")).toHaveTextContent("Loading reasoning");

    rerender(
      <EventCard
        button_id="event-button"
        detail={null}
        detail_error="session file is unavailable"
        detail_loading={false}
        event={reasoningEvent}
        is_expanded
        is_selected={false}
        on_retry_detail={onRetry}
        on_select={vi.fn()}
        on_toggle={vi.fn()}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("session file is unavailable");
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });
});
