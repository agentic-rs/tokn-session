import type { SourceError, SessionSummary, ViewerProvider } from "../lib/types";
import {
  formatRelativeTime,
  groupSessions,
  providerLabel,
  sessionDisplayTitle,
  shortSessionId,
} from "../lib/state";
import { SearchIcon, WarningIcon } from "./Icons";
import { LoadingRows } from "./StateView";

interface SidebarProps {
  sessions: SessionSummary[];
  selected_session_key: string | null;
  enabled_providers: ReadonlySet<ViewerProvider>;
  search: string;
  is_loading: boolean;
  error: string | null;
  source_errors: SourceError[];
  has_more: boolean;
  is_loading_more: boolean;
  on_search_change: (value: string) => void;
  on_provider_toggle: (provider: ViewerProvider) => void;
  on_session_select: (session_key: string) => void;
  on_retry: () => void;
  on_load_more: () => void;
}

const PROVIDER_FILTERS: ViewerProvider[] = ["codex", "pi", "opencode", "dsh"];

export function Sidebar({
  sessions,
  selected_session_key,
  enabled_providers,
  search,
  is_loading,
  error,
  source_errors,
  has_more,
  is_loading_more,
  on_search_change,
  on_provider_toggle,
  on_session_select,
  on_retry,
  on_load_more,
}: SidebarProps) {
  const groups = groupSessions(sessions);
  const noProviders = enabled_providers.size === 0;

  return (
    <aside aria-label="Sessions" className="sidebar">
      <header className="sidebar__header" data-tauri-drag-region>
        <div className="brand-mark" aria-hidden="true">
          <span />
          <span />
          <span />
        </div>
        <div>
          <p className="eyebrow">TOKN</p>
          <h1>Sessions</h1>
        </div>
      </header>

      <div className="sidebar__controls">
        <label className="search-field">
          <span className="sr-only">Search sessions</span>
          <SearchIcon />
          <input
            onChange={(event) => on_search_change(event.currentTarget.value)}
            placeholder="Search sessions"
            spellCheck={false}
            type="search"
            value={search}
          />
          {is_loading && sessions.length > 0 ? <span className="inline-spinner" /> : null}
        </label>

        <div aria-label="Filter by provider" className="provider-filters">
          {PROVIDER_FILTERS.map((provider) => (
            <button
              aria-pressed={enabled_providers.has(provider)}
              className="provider-filter"
              data-provider={provider}
              key={provider}
              onClick={() => on_provider_toggle(provider)}
              type="button"
            >
              <span className="provider-dot" />
              {providerLabel(provider)}
            </button>
          ))}
        </div>
      </div>

      {source_errors.length > 0 ? (
        <details className="source-warning">
          <summary>
            <WarningIcon />
            <span>
              {source_errors.length === 1
                ? `${providerLabel(source_errors[0]!.provider)} could not be read.`
                : `${source_errors.length} providers could not be read.`}
            </span>
          </summary>
          <div className="source-warning__detail">
            <ul>
              {source_errors.map((sourceError) => (
                <li key={sourceError.provider}>
                  <strong>{providerLabel(sourceError.provider)}</strong>
                  <span>{sourceError.message}</span>
                </li>
              ))}
            </ul>
            <button className="text-button" onClick={on_retry} type="button">
              Retry providers
            </button>
          </div>
        </details>
      ) : null}

      <div className="sidebar__list">
        {is_loading && sessions.length === 0 ? <LoadingRows count={6} /> : null}

        {!is_loading && error && sessions.length === 0 ? (
          <div className="sidebar-state" role="alert">
            <strong>Sessions unavailable</strong>
            <span>{error}</span>
            <button className="text-button" onClick={on_retry} type="button">
              Try again
            </button>
          </div>
        ) : null}

        {!is_loading && !error && sessions.length === 0 ? (
          <div className="sidebar-state">
            <strong>{noProviders ? "No providers selected" : "No sessions found"}</strong>
            <span>
              {noProviders
                ? "Enable a provider to browse its sessions."
                : search.trim()
                  ? "Try a different search."
                  : "Known sessions will appear here."}
            </span>
          </div>
        ) : null}

        {groups.map((group) => (
          <section className="session-group" key={group.key}>
            <h2 title={group.project}>{group.project}</h2>
            <div className="session-group__items">
              {group.sessions.map((session) => (
                <button
                  aria-label={`${sessionDisplayTitle(session)}, ${providerLabel(session.provider)} session ${session.session_id}`}
                  aria-current={session.session_key === selected_session_key ? "page" : undefined}
                  className="session-row"
                  data-selected={session.session_key === selected_session_key}
                  key={session.session_key}
                  onClick={() => on_session_select(session.session_key)}
                  title={`${sessionDisplayTitle(session)}\n${session.session_id}`}
                  type="button"
                >
                  <span className="provider-avatar" data-provider={session.provider}>
                    {providerLabel(session.provider).slice(0, 1)}
                  </span>
                  <span className="session-row__body">
                    <span className="session-row__title">{sessionDisplayTitle(session)}</span>
                    <span className="session-row__meta">
                      <span className="session-row__id">{shortSessionId(session.session_id)}</span>
                      <span aria-hidden="true">·</span>
                      <span>{formatRelativeTime(session.timestamp, session.updated_at_ms)}</span>
                      {session.message_count !== null ? (
                        <>
                          <span aria-hidden="true">·</span>
                          <span>{session.message_count} msg</span>
                        </>
                      ) : null}
                    </span>
                  </span>
                </button>
              ))}
            </div>
          </section>
        ))}

        {has_more ? (
          <button
            className="load-more-button"
            disabled={is_loading || is_loading_more}
            onClick={on_load_more}
            type="button"
          >
            {is_loading_more ? "Loading…" : is_loading ? "Updating sessions…" : "Load more sessions"}
          </button>
        ) : null}
      </div>
    </aside>
  );
}
