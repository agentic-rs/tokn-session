import { useEffect, useState } from "react";
import "./App.css";
import { ViewerPage } from "./pages/ViewerPage";
import { isDesktop, RemoteClient, selectMachine, type ConnectionState } from "./lib/transport";

function BrowserViewer() {
  const [endpoint, setEndpoint] = useState("http://127.0.0.1:5558");
  const [token, setToken] = useState("");
  const [client, setClient] = useState<RemoteClient>();
  const [state, setState] = useState<ConnectionState>("connecting");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  useEffect(() => () => selectMachine(), []);

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
      <p>Connect to the viewer API on the machine whose sessions you want to read.</p>
      <label htmlFor="machine-url">API address</label>
      <input id="machine-url" type="url" value={endpoint} disabled={busy} required onChange={(event) => setEndpoint(event.target.value)} />
      <label htmlFor="machine-token">Access token</label>
      <input id="machine-token" type="password" value={token} disabled={busy} autoComplete="off" onChange={(event) => setToken(event.target.value)} />
      <p className="machine-hint">The token is kept only for this connection.</p>
      {error && <p role="alert">{error}</p>}
      <button disabled={busy}>{busy ? "Connecting…" : "Connect"}</button>
    </form>
  </main>;
}
function App() { return isDesktop() ? <ViewerPage /> : <BrowserViewer />; }
export default App;
