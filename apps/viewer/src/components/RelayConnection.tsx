import { useCallback, useEffect, useId, useRef, useState } from "react";
import { configureRelay, getRelayStatus, listenForRelayStatus } from "../lib/tauri";
import { useFloatingPanel } from "../lib/useFloatingPanel";
import { CloseIcon } from "./Icons";
import type { RelayMode, RelaySettings, RelayStatus } from "../lib/types";

const PHASE_LABELS: Record<RelayStatus["phase"], string> = {
  local: "Local history",
  starting: "Connecting",
  connecting: "Connecting",
  live: "Live updates",
  reconnecting: "Reconnecting",
  retrying: "Reconnecting",
  failed: "Connection needs attention",
};

export function RelayConnection() {
  const [status, setStatus] = useState<RelayStatus | null>(null);
  const [settings, setSettings] = useState<RelaySettings>({ mode: "automatic", endpoint: "tcp://127.0.0.1:5557", include_native: false });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const statusRevision = useRef(0);
  const [is_open, setOpen] = useState(false);
  const panel_id = useId();
  const trigger_ref = useRef<HTMLButtonElement>(null);
  const close = useCallback(() => setOpen(false), []);
  const panel_ref = useFloatingPanel(is_open, close, trigger_ref);
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
    if (busy || !status) return;
    setBusy(true);
    setError(null);
    const before = statusRevision.current;
    try {
      const next = await configureRelay({ ...settings, endpoint: settings.endpoint.trim() });
      if (statusRevision.current === before) setStatus(next);
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  }

  const mode = status?.settings.mode ?? "automatic";
  const phase = status?.phase ?? "starting";
  const label = error
    ? status ? "Settings need attention" : "Connection unavailable"
    : mode === "local" ? "Local history" : PHASE_LABELS[phase];

  return (
    <div className="relay-connection">
      <button
        aria-controls={panel_id}
        aria-expanded={is_open}
        aria-haspopup="dialog"
        aria-label={`${label}. Connection settings`}
        className="status-bar__connection"
        data-phase={error ? "failed" : phase}
        onClick={() => {
          if (!is_open && status && !busy) setSettings(status.settings);
          setOpen((open) => !open);
        }}
        ref={trigger_ref}
        title="Connection settings"
        type="button"
      >
        <span aria-hidden="true" className="connection-dot" />
        <span>{label}</span>
      </button>
      {is_open && <div
        aria-labelledby={`${panel_id}-title`}
        aria-modal="false"
        className="connection-panel"
        id={panel_id}
        ref={panel_ref}
        role="dialog"
        tabIndex={-1}
      >
        <header className="notification-center__header">
          <div>
            <h2 id={`${panel_id}-title`}>Connection</h2>
            <p className="connection-panel__summary">{label} · {mode === "automatic" ? "Automatic" : mode === "external" ? "External" : "Local"}</p>
          </div>
          <button aria-label="Close connection settings" className="icon-button" onClick={close} type="button"><CloseIcon /></button>
        </header>
        <form className="relay-settings" onSubmit={(event) => { event.preventDefault(); void save(); }}>
          <label htmlFor={`${panel_id}-mode`}>Data source</label>
          <select id={`${panel_id}-mode`} value={settings.mode} disabled={busy || !status} onChange={(event) => setSettings({ ...settings, mode: event.target.value as RelayMode })}>
            <option value="automatic">Automatic (recommended)</option>
            <option value="external">External Relay</option>
            <option value="local">Local history only</option>
          </select>
          {settings.mode === "external" && <>
            <label htmlFor={`${panel_id}-endpoint`}>Relay endpoint</label>
            <input id={`${panel_id}-endpoint`} value={settings.endpoint} onChange={(event) => setSettings({ ...settings, endpoint: event.target.value })} disabled={busy} spellCheck={false} />
          </>}
          {settings.mode === "automatic" && <label className="relay-native">
            <input type="checkbox" checked={settings.include_native} disabled={busy || !status} onChange={(event) => setSettings({ ...settings, include_native: event.target.checked })} />
            Include native records
          </label>}
          <p>{settings.mode === "automatic"
            ? "Read saved sessions and receive live updates automatically. Native records add provider-specific details to the inspector."
            : settings.mode === "external"
              ? "Connect to a Relay snapshot service running on this machine. You manage that service separately."
              : "Read saved sessions without starting a live Relay connection."}</p>
          {status?.active_endpoint && mode === "external" && <p>Connected to <code>{status.active_endpoint}</code></p>}
          {status && ["reconnecting", "retrying", "failed"].includes(phase) && <p>{mode === "automatic"
            ? phase === "failed"
              ? "Live updates are unavailable. Saved history remains available."
              : "Live updates are reconnecting. Saved history remains available."
            : "Showing the last received data while the connection is unavailable."}</p>}
          {(error || status?.error) && <p role="alert">{error ?? status?.error}</p>}
          <button className="connection-panel__apply" type="submit" disabled={busy || !status}>{busy ? "Saving…" : phase === "failed" && settings.mode === "automatic" ? "Retry" : "Apply"}</button>
        </form>
      </div>}
    </div>
  );
}
