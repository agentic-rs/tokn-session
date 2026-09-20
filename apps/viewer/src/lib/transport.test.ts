import { afterEach, describe, expect, it, vi } from "vitest";
import { parseEvent, RemoteClient, selectMachine, invoke, captureTransport } from "./transport";

const clients: RemoteClient[] = [];
afterEach(() => { for (const client of clients) client.close(); clients.length = 0; selectMachine(); vi.unstubAllGlobals(); vi.useRealTimers(); });
function client() { const result = new RemoteClient("http://machine:5558", "secret"); clients.push(result); return result; }
function stream() {
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  const body = new ReadableStream<Uint8Array>({ start(value) { controller = value; } });
  return { response: new Response(body), send: (value: string) => controller.enqueue(new TextEncoder().encode(value)), end: () => controller.close() };
}

describe("remote viewer transport", () => {
  it("parses named multiline events and ignores heartbeats", () => {
    expect(parseEvent(": keep-alive")).toBeNull();
    expect(parseEvent('event: relay-changed\ndata: {"session_key":null,\ndata: "reset":true}')).toEqual({ event: "relay-changed", payload: { session_key: null, reset: true } });
  });
  it("rejects incompatible APIs before selecting a machine", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json({ version: 9 })));
    await expect(RemoteClient.connect("http://machine", "secret")).rejects.toThrow("unsupported");
  });
  it("uses bearer headers and aborts outstanding requests on machine switch", async () => {
    let signal: AbortSignal | undefined;
    const fetcher = vi.fn((_url: string, options: RequestInit) => new Promise((_resolve, reject) => {
      signal = options.signal as AbortSignal;
      signal.addEventListener("abort", () => reject(new Error("aborted")));
    }));
    vi.stubGlobal("fetch", fetcher);
    selectMachine(client());
    const request = invoke("list_sessions", { request: {} });
    const rejected = expect(request).rejects.toThrow("aborted");
    selectMachine(client());
    await rejected;
    expect(signal?.aborted).toBe(true);
    expect(fetcher.mock.calls[0][1]).toMatchObject({ headers: { Authorization: "Bearer secret" }, credentials: "omit", redirect: "error" });
  });
  it("releases a captured view on its original machine before clearing its credentials", async () => {
    const fetcher = vi.fn().mockImplementation(() => Promise.resolve(Response.json(null)));
    vi.stubGlobal("fetch", fetcher);
    const old = client();
    selectMachine(old);
    const captured = captureTransport();
    captured.on_close(() => { void captured.release("update_session_view", { request: { view_id: "view", session_key: null } }); });
    const next = new RemoteClient("http://next:5558", "next-secret");
    clients.push(next);
    selectMachine(next);
    expect(fetcher).toHaveBeenCalledExactlyOnceWith("http://machine:5558/api/v1/update_session_view", expect.objectContaining({
      headers: { Authorization: "Bearer secret", "Content-Type": "application/json" }, keepalive: true,
    }));
    await expect(captured.invoke("load_event_page", {})).rejects.toThrow("Machine disconnected");
    expect(fetcher).toHaveBeenCalledOnce();
  });
  it("shares one stream, waits for readiness, and refreshes after reconnect", async () => {
    vi.useFakeTimers();
    const first = stream(); const second = stream();
    const fetcher = vi.fn().mockResolvedValueOnce(first.response).mockResolvedValueOnce(second.response);
    vi.stubGlobal("fetch", fetcher);
    const remote = client();
    const changed = vi.fn(); const progress = vi.fn();
    const one = remote.listen("relay-changed", changed);
    const two = remote.listen("session-index-progress", progress);
    first.send("event: rea"); first.send("dy\ndata: {}\n\n");
    const [stopOne, stopTwo] = await Promise.all([one, two]);
    expect(fetcher).toHaveBeenCalledTimes(1);
    first.send('event: session-index-progress\ndata: {"pending":1}\n\n');
    await vi.advanceTimersByTimeAsync(0);
    expect(progress).toHaveBeenCalledWith({ payload: { pending: 1 } });
    first.end();
    await vi.advanceTimersByTimeAsync(1100);
    second.send("event: ready\ndata: {}\n\n");
    await vi.advanceTimersByTimeAsync(0);
    expect(changed).toHaveBeenCalledWith({ payload: { session_key: null, reset: true } });
    stopOne(); stopTwo(); remote.close(); second.end();
  });
});
