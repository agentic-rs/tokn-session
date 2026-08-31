import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SessionSummary } from "../lib/types";
import { Sidebar } from "./Sidebar";

afterEach(cleanup);

function session(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    session_key: "codex:session",
    session_id: "01991dce-7f6a-7000-8000-000000000001",
    parent_session_id: null,
    provider: "codex",
    title: null,
    preview: null,
    project: "Viewer",
    cwd: "/work/viewer",
    updated_at_ms: null,
    timestamp: null,
    agent_path: null,
    message_count: null,
    event_count: null,
    history_status: null,
    ...overrides,
  };
}

function renderSidebar(sessions: SessionSummary[]) {
  return render(
    <Sidebar
      enabled_providers={new Set(["codex"])}
      error={null}
      has_more={false}
      is_loading={false}
      is_loading_more={false}
      on_load_more={vi.fn()}
      on_provider_toggle={vi.fn()}
      on_retry={vi.fn()}
      on_search_change={vi.fn()}
      on_session_select={vi.fn()}
      search=""
      selected_session_key={null}
      sessions={sessions}
      source_errors={[]}
    />,
  );
}

describe("Sidebar session identity", () => {
  it("renders title fallbacks while keeping the full session id discoverable", () => {
    const titledId = "01991dce-7f6a-7000-8000-000000000001";
    const previewId = "abcdef01-2345-6789-abcd-ef0123456789";
    const untitledId = "12345678-90ab-cdef-1234-567890abcdef";

    renderSidebar([
      session({
        session_key: "codex:titled",
        session_id: titledId,
        title: "Provider title",
        preview: "First prompt should not win",
      }),
      session({
        session_key: "codex:preview",
        session_id: previewId,
        preview: "First user prompt",
      }),
      session({
        session_key: "codex:untitled",
        session_id: untitledId,
        title: "  ",
        preview: "\n\t",
      }),
    ]);

    const titled = screen.getByRole("button", {
      name: `Provider title, Codex session ${titledId}`,
    });
    expect(within(titled).getByText("Provider title")).toHaveClass("session-row__title");
    expect(within(titled).queryByText("First prompt should not win")).not.toBeInTheDocument();
    expect(within(titled).getByText("01991dce…")).toHaveClass("session-row__id");
    expect(within(titled).queryByText(titledId)).not.toBeInTheDocument();
    expect(titled).toHaveAttribute("title", `Provider title\n${titledId}`);

    const preview = screen.getByRole("button", {
      name: `First user prompt, Codex session ${previewId}`,
    });
    expect(within(preview).getByText("First user prompt")).toHaveClass("session-row__title");
    expect(within(preview).getByText("abcdef01…")).toHaveClass("session-row__id");
    expect(preview).toHaveAttribute("title", `First user prompt\n${previewId}`);

    const untitled = screen.getByRole("button", {
      name: `Untitled session, Codex session ${untitledId}`,
    });
    expect(within(untitled).getByText("Untitled session")).toHaveClass("session-row__title");
    expect(within(untitled).getByText("12345678…")).toHaveClass("session-row__id");
    expect(untitled).toHaveAttribute("title", `Untitled session\n${untitledId}`);
  });
});
