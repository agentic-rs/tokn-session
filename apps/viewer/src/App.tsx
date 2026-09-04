import { useEffect, useState } from "react";
import "./App.css";
import { ViewerPage } from "./pages/ViewerPage";
import { isDesktop, RemoteClient, selectMachine, type ConnectionState } from "./lib/transport";

function BrowserViewer({ initial_token }: { initial_token?: string }) {
  const [endpoint, setEndpoint] = useState(() => window.location.origin);
  const [token, setToken] = useState("");
  const [client, setClient] = useState<RemoteClient>();
  const [state, setState] = useState<ConnectionState>("connecting");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  useEffect(() => () => selectMachine(), []);
  useEffect(() => {
    if (!initial_token) return;
    let cancelled = false;
    setBusy(true);
    void RemoteClient.connect(window.location.origin, initial_token).then((next) => {
      if (cancelled) { next.close(); return; }
      next.setStateListener(setState);
      selectMachine(next);
      setClient(next);
    }).catch((error: unknown) => {
      if (!cancelled) setError(error instanceof Error ? error.message : String(error));
    }).finally(() => { if (!cancelled) setBusy(false); });
    return () => { cancelled = true; };
  }, [initial_token]);

  async function connect() {
    setBusy(true); setError(undefined);
    try {
      const next = await RemoteClient.connect(endpoint.trim(), token);
      next.setStateListener(setState);
      selectMachine(next); setClient(next); setToken("");
    } catch (error) { setError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  }
  if (client) return <div className="remote-viewer">
    <div className="machine-bar">
      <span>{client.endpoint} · {state === "reconnecting" ? "Reconnecting · showing last received data" : state}</span>
      <button onClick={() => { selectMachine(); setClient(undefined); setState("connecting"); }}>Change machine</button>
    </div>
    <ViewerPage remote />
  </div>;
  return <main className="machine-connect">
    <form onSubmit={(event) => { event.preventDefault(); void connect(); }}>
      <h1>Session viewer</h1>
      <p>Connect to the viewer server on the machine whose sessions you want to read.</p>
      <label htmlFor="machine-url">Viewer address</label>
      <input id="machine-url" type="url" value={endpoint} disabled={busy} required onChange={(event) => setEndpoint(event.target.value)} />
      <label htmlFor="machine-token">Access token</label>
      <input id="machine-token" type="password" value={token} disabled={busy} autoComplete="off" onChange={(event) => setToken(event.target.value)} />
      <p className="machine-hint">The token is kept only for this connection.</p>
      {error && <p role="alert">{error}</p>}
      <button disabled={busy}>{busy ? "Connecting…" : "Connect"}</button>
    </form>
  </main>;
}
function App({ initial_token }: { initial_token?: string }) {
  return isDesktop() ? <ViewerPage /> : <BrowserViewer initial_token={initial_token} />;
}
export default App;
