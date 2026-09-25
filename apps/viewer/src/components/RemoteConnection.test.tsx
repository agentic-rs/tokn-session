import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { RemoteConnection } from "./RemoteConnection";

afterEach(cleanup);

it("keeps host identity visible and shows current recovery state and actions on demand", () => {
  const change_host = vi.fn();
  const view = (state: "connected" | "reconnecting") => <RemoteConnection name="Workstation · View only" state={state}>
    <button onClick={change_host}>Change host</button>
  </RemoteConnection>;
  const { rerender } = render(view("connected"));
  expect(screen.getByText("Connected · Workstation · View only")).toBeInTheDocument();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  rerender(view("reconnecting"));
  fireEvent.click(screen.getByRole("button", { name: /connection settings/i }));
  expect(screen.getByRole("dialog", { name: "Connection" })).toHaveFocus();
  expect(screen.getByText("Reconnecting · showing last received data")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Change host" }));
  expect(change_host).toHaveBeenCalledOnce();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: /connection settings/i })).toHaveFocus();
});
