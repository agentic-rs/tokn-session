import { useEffect, useRef, useState } from "react";
import { configureRelay, getRelayStatus, listenForRelayStatus } from "../lib/tauri";
import type { RelayStatus } from "../lib/types";

export function RelayConnection() {
  const [status, setStatus] = useState<RelayStatus | null>(null);
  const [endpoint, setEndpoint] = useState("tcp://127.0.0.1:5557");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const statusRevision = useRef(0);
  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    let latest: RelayStatus | null = null;
    void listenForRelayStatus((next) => {
      statusRevision.current += 1;
      latest = next;
      if (!disposed) setStatus(next);
    }).then(async (unlisten) => {
      if (disposed) { unlisten(); return; }
      stop = unlisten;
      const before = statusRevision.current;
      const next = await getRelayStatus();
      if (!disposed) {
        if (statusRevision.current === before) setStatus(next);
        setEndpoint((latest ?? next).settings.endpoint);
      }
    }).catch(() => { if (!disposed) setError("Relay settings are available in the desktop app."); });
    return () => { disposed = true; stop?.(); };
  }, []);

  async function save(enabled: boolean) {
    setBusy(true); setError(null);
    const before = statusRevision.current;
    try {
      const next = await configureRelay({ endpoint: endpoint.trim(), enabled });
      if (statusRevision.current === before) setStatus(next);
    }
    catch (error) { setError(String(error)); }
    finally { setBusy(false); }
  }

  return (
    <details className="relay-connection">
      <summary>Relay · {status?.phase ?? "local history"}</summary>
      <div className="relay-settings">
        <label htmlFor="relay-endpoint">Relay endpoint</label>
        <input id="relay-endpoint" value={endpoint} onChange={(event) => setEndpoint(event.target.value)} disabled={busy} spellCheck={false} />
        <button type="button" disabled={busy} onClick={() => void save(true)}>{busy ? "Saving…" : "Connect"}</button>
        <button type="button" disabled={busy || !status?.settings.enabled} onClick={() => void save(false)}>Disconnect</button>
        <p>Connect to an independently started local <code>tokn-session-relay serve</code>. Other providers continue using local history. Disconnect keeps cached Relay sessions until you restart the viewer.</p>
        {status && <p>Native records: {status.native ? "available" : "not enabled on Relay"}. {status.phase === "reconnecting" ? "Keeping the last received snapshot while reconnecting." : ""}</p>}
        {(error || status?.error) && <p role="alert">{error ?? status?.error}</p>}
      </div>
    </details>
  );
}
