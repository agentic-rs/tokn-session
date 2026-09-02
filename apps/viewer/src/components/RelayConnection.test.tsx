import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { configureRelay, getRelayStatus, listenForRelayStatus } from "../lib/tauri";
import type { RelayStatus } from "../lib/types";
import { RelayConnection } from "./RelayConnection";

vi.mock("../lib/tauri", () => ({ configureRelay: vi.fn(), getRelayStatus: vi.fn(), listenForRelayStatus: vi.fn() }));
const disconnected: RelayStatus = { settings: { endpoint: "tcp://127.0.0.1:9557", enabled: false }, phase: "disconnected", native: false, error: null };
beforeEach(() => {
  vi.mocked(getRelayStatus).mockResolvedValue(disconnected);
  vi.mocked(listenForRelayStatus).mockResolvedValue(vi.fn());
  vi.mocked(configureRelay).mockReset();
});
afterEach(cleanup);

it("loads saved endpoint and saves trimmed connection settings", async () => {
  vi.mocked(configureRelay).mockResolvedValue({ ...disconnected, phase: "connecting", settings: { ...disconnected.settings, enabled: true } });
  render(<RelayConnection />);
  const input = screen.getByLabelText("Relay endpoint");
  await waitFor(() => expect(input).toHaveValue(disconnected.settings.endpoint));
  fireEvent.change(input, { target: { value: " tcp://127.0.0.1:9558 " } });
  fireEvent.click(screen.getByText("Connect"));
  await waitFor(() => expect(configureRelay).toHaveBeenCalledWith({ endpoint: "tcp://127.0.0.1:9558", enabled: true }));
});

it("keeps a newer live status when an older configuration reply arrives", async () => {
  let emit: ((status: RelayStatus) => void) | undefined;
  vi.mocked(listenForRelayStatus).mockImplementation((handler) => { emit = handler; return Promise.resolve(vi.fn()); });
  let resolve!: (status: RelayStatus) => void;
  vi.mocked(configureRelay).mockReturnValue(new Promise((done) => { resolve = done; }));
  render(<RelayConnection />);
  await waitFor(() => expect(screen.getByLabelText("Relay endpoint")).toHaveValue(disconnected.settings.endpoint));
  fireEvent.click(screen.getByText("Connect"));
  act(() => emit?.({ ...disconnected, phase: "live", native: true }));
  await act(async () => resolve({ ...disconnected, phase: "connecting" }));
  expect(screen.getByText("Relay · live")).toBeInTheDocument();
});

it("shows validation errors without claiming it connected", async () => {
  vi.mocked(configureRelay).mockRejectedValue("Relay endpoint must be loopback");
  render(<RelayConnection />);
  await waitFor(() => expect(screen.getByLabelText("Relay endpoint")).toHaveValue(disconnected.settings.endpoint));
  fireEvent.click(screen.getByText("Connect"));
  await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("loopback"));
  expect(screen.getByText("Relay · disconnected")).toBeInTheDocument();
});
