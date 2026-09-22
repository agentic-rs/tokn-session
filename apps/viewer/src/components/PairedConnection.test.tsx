import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RemoteClient } from "../lib/transport";
import { PairedConnection } from "./PairedConnection";

vi.mock("../pages/ViewerPage", () => ({ ViewerPage: () => <div>Remote sessions</div> }));
const host_id = "11111111-1111-4111-8111-111111111111";
const saved_host = { host_id, host_public_key: "saved-pin" };
const selection = { host_id, connection_id: "connection-one" };
const status = { hosts: [saved_host], selected: null, hub_url: "https://hub.example.com/" };

beforeEach(() => { vi.spyOn(globalThis, "fetch").mockResolvedValue(Response.json(status)); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it("opens the remembered host with its connection path and without requesting an authenticator code", async () => {
  vi.mocked(fetch).mockImplementation(async () => Response.json({ ...status, selected: selection }));
  const connect = vi.spyOn(RemoteClient, "connect").mockImplementation(async (endpoint, token) => new RemoteClient(endpoint, token));
  render(<StrictMode><PairedConnection initial_token="local-token" /></StrictMode>);
  expect(await screen.findByText("Remote sessions")).toBeInTheDocument();
  expect(connect).toHaveBeenCalledWith(`${window.location.origin}/paired/connection-one`, "local-token", expect.any(AbortSignal));
  expect(screen.queryByLabelText("Authenticator code")).not.toBeInTheDocument();
  for (const [url, options] of vi.mocked(fetch).mock.calls) {
    expect(url).toBe("/api/local/status");
    expect(options?.headers).toEqual({ Authorization: "Bearer local-token" });
    expect(options?.body).toBeUndefined();
  }
});

it("submits pairing only to the local endpoint, clears the code, and never retries failed pairing automatically", async () => {
  vi.mocked(fetch).mockImplementation(async (url) => {
    if (url === "/api/local/pair") return Response.json({ error: "Wait for a fresh code" }, { status: 502 });
    return Response.json({ ...status, hosts: [] });
  });
  render(<PairedConnection initial_token="local-token" />);
  await waitFor(() => expect(screen.getByLabelText("Host ID")).toBeEnabled());
  fireEvent.change(screen.getByLabelText("Host ID"), { target: { value: host_id } });
  fireEvent.change(screen.getByLabelText("Authenticator code"), { target: { value: "123456" } });
  fireEvent.click(screen.getByRole("button", { name: "Pair and connect" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Wait for a fresh code");
  expect(screen.getByLabelText("Authenticator code")).toHaveValue("");
  const pairing = vi.mocked(fetch).mock.calls.filter(([url]) => url === "/api/local/pair");
  expect(pairing).toHaveLength(1);
  expect(JSON.parse(pairing[0][1]?.body as string)).toEqual({ host_id, code: "123456" });
  expect(pairing[0][1]).toMatchObject({ credentials: "omit", redirect: "error", method: "POST" });
  expect(vi.mocked(fetch).mock.calls.every(([url]) => String(url).startsWith("/api/local/"))).toBe(true);
});

it("disconnects the old viewer and obtains a new selection before reconnecting", async () => {
  const old_client = new RemoteClient(`${window.location.origin}/paired/old-connection`, "local-token");
  const close = vi.spyOn(old_client, "close");
  vi.spyOn(RemoteClient, "connect").mockResolvedValueOnce(old_client)
    .mockResolvedValueOnce(new RemoteClient(`${window.location.origin}/paired/new-connection`, "local-token"));
  vi.mocked(fetch).mockImplementation(async (url) => {
    if (url === "/api/local/disconnect") return new Response(null, { status: 204 });
    if (url === "/api/local/select") return Response.json({ selected: { ...selection, connection_id: "new-connection" } });
    return Response.json({ ...status, selected: { ...selection, connection_id: "old-connection" } });
  });
  render(<PairedConnection initial_token="local-token" />);
  expect(await screen.findByText("Remote sessions")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Change host" }));
  await waitFor(() => expect(screen.getByRole("button", { name: `Open ${host_id}` })).toBeEnabled());
  expect(close).toHaveBeenCalled();
  expect(screen.queryByText("Remote sessions")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: `Open ${host_id}` }));
  expect(await screen.findByText("Remote sessions")).toBeInTheDocument();
  expect(RemoteClient.connect).toHaveBeenLastCalledWith(`${window.location.origin}/paired/new-connection`, "local-token", expect.any(AbortSignal));
});

it("requires the local connection link before requesting any saved hosts", () => {
  render(<PairedConnection />);
  expect(screen.getByText(/Open the connection link/)).toBeInTheDocument();
  expect(fetch).not.toHaveBeenCalled();
});
