import { useEffect, useRef, useState } from "react";
import { configureRelay, getRelayStatus, listenForRelayStatus } from "../lib/tauri";
import type { RelayMode, RelaySettings, RelayStatus } from "../lib/types";

export function RelayConnection() {
  const [status, setStatus] = useState<RelayStatus | null>(null);
  const [settings, setSettings] = useState<RelaySettings>({ mode: "automatic", endpoint: "tcp://127.0.0.1:5557", include_native: false });
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
        setSettings((latest ?? next).settings);
      }
    }).catch(() => { if (!disposed) setError("Relay settings are available in the desktop app."); });
    return () => { disposed = true; stop?.(); };
  }, []);

  async function save() {
    setBusy(true); setError(null);
    const before = statusRevision.current;
    try {
      const next = await configureRelay({ ...settings, endpoint: settings.endpoint.trim() });
      if (statusRevision.current === before) setStatus(next);
    }
    catch (error) { setError(String(error)); }
    finally { setBusy(false); }
  }

  return (
    <details className="relay-connection">
      <summary>{status?.settings.mode === "local" ? "Local history" : `Relay · ${status?.settings.mode ?? "automatic"} · ${status?.phase ?? "starting"}`}</summary>
      <div className="relay-settings">
        <label htmlFor="relay-mode">Data source</label>
        <select id="relay-mode" value={settings.mode} disabled={busy || !status} onChange={(event) => setSettings({ ...settings, mode: event.target.value as RelayMode })}>
          <option value="automatic">Automatic Relay</option>
          <option value="external">External Relay</option>
          <option value="local">Local history only</option>
        </select>
        {settings.mode === "external" && <>
          <label htmlFor="relay-endpoint">Relay endpoint</label>
          <input id="relay-endpoint" value={settings.endpoint} onChange={(event) => setSettings({ ...settings, endpoint: event.target.value })} disabled={busy} spellCheck={false} />
        </>}
        {settings.mode === "automatic" && <label className="relay-native">
          <input type="checkbox" checked={settings.include_native} disabled={busy || !status} onChange={(event) => setSettings({ ...settings, include_native: event.target.checked })} />
          Include native records
        </label>}
        <button type="button" disabled={busy || !status} onClick={() => void save()}>{busy ? "Saving…" : status?.phase === "failed" && settings.mode === "automatic" ? "Retry" : "Apply"}</button>
        <p>{settings.mode === "automatic"
          ? "Relay starts with this app through a private stdio pipe and stops when the app exits. Native records are optional and may contain sensitive data."
          : settings.mode === "external"
            ? "Connect to an independently started local tokn-viewer-api snapshot. This app never stops an external Relay."
            : "Read provider history directly. Applying Local mode clears Relay snapshots and stops any app-owned Relay."}</p>
        {status?.active_endpoint && <p>Active endpoint: <code>{status.active_endpoint}</code></p>}
        {status && status.settings.mode !== "local" && <p>Native records: {status.native ? "available" : "not available"}. Other providers use local history.</p>}
        {status && ["reconnecting", "retrying", "failed"].includes(status.phase) && <p>Keeping any last received snapshots; no automatic fallback to local reads.</p>}
        {(error || status?.error) && <p role="alert">{error ?? status?.error}</p>}
      </div>
    </details>
  );
}
