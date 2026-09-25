import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { configureRelay, getRelayStatus, listenForRelayStatus } from "../lib/tauri";
import type { RelayStatus } from "../lib/types";
import { RelayConnection } from "./RelayConnection";

vi.mock("../lib/tauri", () => ({ configureRelay: vi.fn(), getRelayStatus: vi.fn(), listenForRelayStatus: vi.fn() }));
const disconnected: RelayStatus = { settings: { endpoint: "tcp://127.0.0.1:9557", mode: "external", include_native: false }, active_endpoint: null, phase: "connecting", native: false, error: null };
beforeEach(() => {
  vi.mocked(getRelayStatus).mockResolvedValue(disconnected);
  vi.mocked(listenForRelayStatus).mockResolvedValue(vi.fn());
  vi.mocked(configureRelay).mockReset();
});
afterEach(cleanup);

async function openConnection() {
  fireEvent.click(screen.getByRole("button", { name: /connection settings/i }));
  await waitFor(() => expect(screen.getByLabelText("Data source")).toBeEnabled());
}

it("loads saved endpoint and saves trimmed connection settings", async () => {
  vi.mocked(configureRelay).mockResolvedValue(disconnected);
  render(<RelayConnection />);
  await openConnection();
  const input = await screen.findByLabelText("Relay endpoint");
  await waitFor(() => expect(input).toHaveValue(disconnected.settings.endpoint));
  fireEvent.change(input, { target: { value: " tcp://127.0.0.1:9558 " } });
  fireEvent.click(screen.getByText("Apply"));
  await waitFor(() => expect(configureRelay).toHaveBeenCalledWith({ endpoint: "tcp://127.0.0.1:9558", mode: "external", include_native: false }));
});

it("keeps a newer live status when an older configuration reply arrives", async () => {
  let emit: ((status: RelayStatus) => void) | undefined;
  vi.mocked(listenForRelayStatus).mockImplementation((handler) => { emit = handler; return Promise.resolve(vi.fn()); });
  let resolve!: (status: RelayStatus) => void;
  vi.mocked(configureRelay).mockReturnValue(new Promise((done) => { resolve = done; }));
  render(<RelayConnection />);
  await openConnection();
  await waitFor(() => expect(screen.getByLabelText("Relay endpoint")).toHaveValue(disconnected.settings.endpoint));
  fireEvent.click(screen.getByText("Apply"));
  act(() => emit?.({ ...disconnected, phase: "live", native: true }));
  await act(async () => resolve({ ...disconnected, phase: "connecting" }));
  expect(screen.getByRole("button", { name: "Live updates. Connection settings" })).toBeInTheDocument();
});

it("shows validation errors without claiming it connected", async () => {
  vi.mocked(configureRelay).mockRejectedValue("Relay endpoint must be loopback");
  render(<RelayConnection />);
  await openConnection();
  await waitFor(() => expect(screen.getByLabelText("Relay endpoint")).toHaveValue(disconnected.settings.endpoint));
  fireEvent.click(screen.getByText("Apply"));
  await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("loopback"));
  expect(screen.getByRole("button", { name: "Settings need attention. Connection settings" })).toBeInTheDocument();
});

it("offers native opt-in without exposing managed connection internals", async () => {
  const automatic: RelayStatus = { ...disconnected, settings: { ...disconnected.settings, mode: "automatic" }, active_endpoint: "tcp://127.0.0.1:12345", phase: "live" };
  vi.mocked(getRelayStatus).mockResolvedValue(automatic);
  vi.mocked(configureRelay).mockResolvedValue(automatic);
  render(<RelayConnection />);
  await openConnection();
  expect(screen.getByRole("button", { name: "Live updates. Connection settings" })).toBeInTheDocument();
  expect(screen.queryByLabelText("Relay endpoint")).not.toBeInTheDocument();
  expect(screen.queryByText(automatic.active_endpoint!)).not.toBeInTheDocument();
  fireEvent.click(screen.getByLabelText("Include native records"));
  fireEvent.click(screen.getByText("Apply"));
  await waitFor(() => expect(configureRelay).toHaveBeenCalledWith({ ...automatic.settings, include_native: true }));
});

it("switches explicitly to local without a confusing disconnected label", async () => {
  vi.mocked(configureRelay).mockResolvedValue({ ...disconnected, settings: { ...disconnected.settings, mode: "local" }, phase: "local" });
  render(<RelayConnection />);
  await openConnection();
  await screen.findByLabelText("Relay endpoint");
  fireEvent.change(screen.getByLabelText("Data source"), { target: { value: "local" } });
  fireEvent.click(screen.getByText("Apply"));
  await screen.findByText("Local history");
  expect(configureRelay).toHaveBeenCalledWith({ ...disconnected.settings, mode: "local" });
});

it("offers a retry after bounded startup failures without dropping the saved endpoint", async () => {
  const failed: RelayStatus = { ...disconnected, settings: { ...disconnected.settings, mode: "automatic" }, phase: "failed", error: "Relay startup timed out" };
  vi.mocked(getRelayStatus).mockResolvedValue(failed);
  vi.mocked(configureRelay).mockResolvedValue({ ...failed, phase: "starting", error: null });
  render(<RelayConnection />);
  await openConnection();
  fireEvent.click(await screen.findByText("Retry"));
  await waitFor(() => expect(configureRelay).toHaveBeenCalledWith(failed.settings));
  await screen.findByRole("button", { name: "Connecting. Connection settings" });
});

it("opens a floating dialog on demand and dismisses without losing status", async () => {
  render(<RelayConnection />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByLabelText("Data source")).not.toBeInTheDocument();
  await openConnection();
  expect(screen.getByRole("dialog", { name: "Connection" })).toHaveFocus();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: /connection settings/i })).toHaveFocus();
  await openConnection();
  fireEvent.pointerDown(document.body);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("describes automatic recovery without claiming historical reads are unavailable", async () => {
  vi.mocked(getRelayStatus).mockResolvedValue({ ...disconnected, settings: { ...disconnected.settings, mode: "automatic" }, phase: "reconnecting" });
  render(<RelayConnection />);
  await openConnection();
  expect(screen.getByText("Live updates are reconnecting. Saved history remains available.")).toBeInTheDocument();
});
