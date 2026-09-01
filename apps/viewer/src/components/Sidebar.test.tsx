import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SessionSummary } from "../lib/types";
import { Sidebar } from "./Sidebar";

afterEach(cleanup);

function session(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    session_key: "codex:session",
    session_id: "01991dce-7f6a-7000-8000-000000000001",
    parent_session_id: null,
    is_subagent: false,
    provider: "codex",
    title: null,
    preview: null,
    project: "Viewer",
    cwd: "/work/viewer",
    updated_at_ms: null,
    timestamp: null,
    agent_path: null,
    agent_nickname: null,
    agent_role: null,
    child_count: 0,
    message_count: null,
    event_count: null,
    history_status: null,
    has_unread: false,
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
      on_children_load={vi.fn()}
      on_children_load_more={vi.fn()}
      on_children_retry={vi.fn()}
      on_load_more={vi.fn()}
      on_provider_toggle={vi.fn()}
      on_retry={vi.fn()}
      on_search_change={vi.fn()}
      on_session_select={vi.fn()}
      search=""
      session_children={new Map()}
      selected_session_key={null}
      sessions={sessions}
      source_errors={[]}
    />,
  );
}

describe("Sidebar session identity", () => {
  it("offers WorkBuddy as a provider filter", () => {
    renderSidebar([]);

    const filter = screen.getByRole("button", { name: "WorkBuddy" });
    expect(filter).toHaveAttribute("data-provider", "workbuddy");
    expect(filter).toHaveAttribute("aria-pressed", "false");
  });

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

  it("loads missing child metadata only after an expansion commits", async () => {
    const onChildrenLoad = vi.fn();
    const parent = session({
      session_key: "codex:parent",
      session_id: "parent-0000",
      title: "Root task",
      child_count: 1,
    });

    render(
      <Sidebar
        enabled_providers={new Set(["codex"])}
        error={null}
        has_more={false}
        is_loading={false}
        is_loading_more={false}
        on_children_load={onChildrenLoad}
        on_children_load_more={vi.fn()}
        on_children_retry={vi.fn()}
        on_load_more={vi.fn()}
        on_provider_toggle={vi.fn()}
        on_retry={vi.fn()}
        on_search_change={vi.fn()}
        on_session_select={vi.fn()}
        search=""
        session_children={new Map()}
        selected_session_key={null}
        sessions={[parent]}
        source_errors={[]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Show 1 subagent for Root task" }));
    await waitFor(() => expect(onChildrenLoad).toHaveBeenCalledWith(parent.session_key));
  });

  it("renders cached nested subagents as independently selectable tree items", () => {
    const onChildrenLoad = vi.fn();
    const onSessionSelect = vi.fn();
    const parent = session({
      session_key: "codex:parent",
      session_id: "parent-0000",
      title: "Root task",
      child_count: 1,
    });
    const child = session({
      session_key: "codex:child",
      session_id: "child-0000",
      parent_session_id: "parent-0000",
      is_subagent: true,
      title: null,
      agent_nickname: "Hubble",
      agent_path: "/root/researcher",
    });

    render(
      <Sidebar
        enabled_providers={new Set(["codex"])}
        error={null}
        has_more={false}
        is_loading={false}
        is_loading_more={false}
        on_children_load={onChildrenLoad}
        on_children_load_more={vi.fn()}
        on_children_retry={vi.fn()}
        on_load_more={vi.fn()}
        on_provider_toggle={vi.fn()}
        on_retry={vi.fn()}
        on_search_change={vi.fn()}
        on_session_select={onSessionSelect}
        search=""
        session_children={new Map([[
          parent.session_key,
          {
            sessions: [child],
            next_cursor: null,
            is_loading: false,
            is_loading_more: false,
            error: null,
          },
        ]])}
        selected_session_key={null}
        sessions={[parent]}
        source_errors={[]}
      />,
    );

    expect(screen.queryByRole("button", { name: /subagent Hubble/i })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show 1 subagent for Root task" }));
    expect(onChildrenLoad).not.toHaveBeenCalled();

    const childRow = screen.getByRole("button", {
      name: `subagent Hubble, Codex session ${child.session_id}`,
    });
    expect(childRow).toHaveTextContent("/root/researcher");
    fireEvent.click(childRow);
    expect(onSessionSelect).toHaveBeenCalledWith(child.session_key);
  });
});

describe("Sidebar unread activity", () => {
  it("renders an accessible indicator for a directly unread session", () => {
    const unread = session({
      session_key: "codex:unread",
      session_id: "unread-0000",
      title: "Needs attention",
      has_unread: true,
    });

    renderSidebar([unread]);

    const row = screen.getByRole("button", {
      name: "Needs attention, Codex session unread-0000, unread updates",
    });
    const dot = within(row).getByRole("img", { name: "Unread updates" });

    expect(row).toHaveAttribute("data-unread", "true");
    expect(dot).toHaveClass("session-row__unread-dot");
    expect(dot).toHaveAttribute("data-unread-source", "direct");
  });

  it("renders unread indicators on a parent and its loaded unread subagent", () => {
    const parent = session({
      session_key: "codex:parent",
      session_id: "parent-0000",
      title: "Root task",
      child_count: 1,
      has_unread_descendant: true,
    });
    const child = session({
      session_key: "codex:child",
      session_id: "child-0000",
      parent_session_id: parent.session_id,
      is_subagent: true,
      agent_nickname: "Hubble",
      has_unread: true,
    });

    render(
      <Sidebar
        enabled_providers={new Set(["codex"])}
        error={null}
        has_more={false}
        is_loading={false}
        is_loading_more={false}
        on_children_load={vi.fn()}
        on_children_load_more={vi.fn()}
        on_children_retry={vi.fn()}
        on_load_more={vi.fn()}
        on_provider_toggle={vi.fn()}
        on_retry={vi.fn()}
        on_search_change={vi.fn()}
        on_session_select={vi.fn()}
        search=""
        session_children={new Map([[
          parent.session_key,
          {
            sessions: [child],
            next_cursor: null,
            is_loading: false,
            is_loading_more: false,
            error: null,
          },
        ]])}
        selected_session_key={null}
        sessions={[parent]}
        source_errors={[]}
      />,
    );

    const parentRow = screen.getByRole("button", {
      name: "Root task, Codex session parent-0000, unread updates in a subagent",
    });
    expect(within(parentRow).getByRole("img", { name: "Unread updates in a subagent" }))
      .toHaveAttribute("data-unread-source", "descendant");

    fireEvent.click(screen.getByRole("button", { name: "Show 1 subagent for Root task" }));

    const childRow = screen.getByRole("button", {
      name: "subagent Hubble, Codex session child-0000, unread updates",
    });
    expect(within(childRow).getByRole("img", { name: "Unread updates" }))
      .toHaveAttribute("data-unread-source", "direct");
  });
});
