import { useCallback, useEffect, useState } from "react";
import type {
  SessionChildrenState,
  SourceError,
  SessionSummary,
  ViewerProvider,
} from "../lib/types";
import {
  formatRelativeTime,
  groupSessions,
  knownSessionAncestors,
  providerLabel,
  sessionDisplayTitle,
  shortSessionId,
  subagentDetail,
} from "../lib/state";
import { ChevronIcon, SearchIcon, WarningIcon } from "./Icons";
import { LoadingRows } from "./StateView";

interface SidebarProps {
  sessions: SessionSummary[];
  session_children: ReadonlyMap<string, SessionChildrenState>;
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
  on_children_load: (parent_session_key: string) => void;
  on_children_retry: (parent_session_key: string) => void;
  on_children_load_more: (parent_session_key: string) => void;
  on_retry: () => void;
  on_load_more: () => void;
}

interface SessionBranchProps {
  session: SessionSummary;
  depth: number;
  expanded_session_keys: ReadonlySet<string>;
  session_children: ReadonlyMap<string, SessionChildrenState>;
  selected_session_key: string | null;
  on_toggle: (session_key: string) => void;
  on_children_load: (parent_session_key: string) => void;
  on_session_select: (session_key: string) => void;
  on_children_retry: (parent_session_key: string) => void;
  on_children_load_more: (parent_session_key: string) => void;
}

const PROVIDER_FILTERS: ViewerProvider[] = ["codex", "pi", "opencode", "zcode", "workbuddy", "dsh"];

function subagentCountLabel(count: number): string {
  return `${count} subagent${count === 1 ? "" : "s"}`;
}

function SessionBranch({
  session,
  depth,
  expanded_session_keys,
  session_children,
  selected_session_key,
  on_toggle,
  on_children_load,
  on_session_select,
  on_children_retry,
  on_children_load_more,
}: SessionBranchProps) {
  const hasChildren = session.child_count > 0;
  const isExpanded = expanded_session_keys.has(session.session_key);
  const childrenState = session_children.get(session.session_key);
  const children = childrenState?.sessions ?? [];
  const relationship = depth > 0 ? subagentDetail(session) : null;
  const title = sessionDisplayTitle(session);
  const sessionDescription = depth > 0 ? `subagent ${title}` : title;
  const hasUnread = session.has_unread || session.has_unread_descendant === true;
  const unreadLabel = session.has_unread
    ? "Unread updates"
    : "Unread updates in a subagent";

  useEffect(() => {
    if (hasChildren && isExpanded && !childrenState) {
      on_children_load(session.session_key);
    }
  }, [childrenState, hasChildren, isExpanded, on_children_load, session.session_key]);

  return (
    <div className="session-tree__branch" data-depth={depth}>
      <div className="session-tree__row">
        {hasChildren ? (
          <button
            aria-expanded={isExpanded}
            aria-label={`${isExpanded ? "Hide" : "Show"} ${subagentCountLabel(session.child_count)} for ${title}`}
            className="session-tree__toggle"
            onClick={() => on_toggle(session.session_key)}
            type="button"
          >
            <ChevronIcon className={isExpanded ? "is-expanded" : undefined} />
          </button>
        ) : (
          <span aria-hidden="true" className="session-tree__toggle-spacer" />
        )}
        <button
          aria-current={session.session_key === selected_session_key ? "page" : undefined}
          aria-label={`${sessionDescription}, ${providerLabel(session.provider)} session ${session.session_id}${hasUnread ? `, ${unreadLabel.toLowerCase()}` : ""}`}
          className="session-row"
          data-selected={session.session_key === selected_session_key}
          data-subagent={depth > 0}
          data-unread={hasUnread || undefined}
          onClick={() => on_session_select(session.session_key)}
          title={`${title}\n${session.session_id}`}
          type="button"
        >
          <span className="provider-avatar" data-provider={session.provider}>
            {providerLabel(session.provider).slice(0, 1)}
            {hasUnread ? (
              <span
                aria-label={unreadLabel}
                className="session-row__unread-dot"
                data-unread-source={session.has_unread ? "direct" : "descendant"}
                role="img"
              />
            ) : null}
          </span>
          <span className="session-row__body">
            <span className="session-row__title">{title}</span>
            <span className="session-row__meta">
              <span className="session-row__id">{shortSessionId(session.session_id)}</span>
              <span aria-hidden="true">·</span>
              <span>{formatRelativeTime(session.timestamp, session.updated_at_ms)}</span>
              {relationship ? (
                <>
                  <span aria-hidden="true">·</span>
                  <span className="session-row__relationship" title={relationship}>{relationship}</span>
                </>
              ) : null}
              {session.child_count > 0 ? (
                <>
                  <span aria-hidden="true">·</span>
                  <span>{subagentCountLabel(session.child_count)}</span>
                </>
              ) : null}
              {session.message_count !== null ? (
                <>
                  <span aria-hidden="true">·</span>
                  <span>{session.message_count} msg</span>
                </>
              ) : null}
            </span>
          </span>
        </button>
      </div>

      {hasChildren && isExpanded ? (
        <div className="session-tree__children">
          {childrenState?.is_loading && children.length === 0 ? (
            <div className="session-tree__state">
              <span className="inline-spinner" />
              Loading subagents…
            </div>
          ) : null}

          {childrenState && !childrenState.is_loading && !childrenState.error && children.length === 0 ? (
            <div className="session-tree__state">No current subagents.</div>
          ) : null}

          {children.map((child) => (
            <SessionBranch
              depth={depth + 1}
              expanded_session_keys={expanded_session_keys}
              key={child.session_key}
              on_children_load={on_children_load}
              on_children_load_more={on_children_load_more}
              on_children_retry={on_children_retry}
              on_session_select={on_session_select}
              on_toggle={on_toggle}
              selected_session_key={selected_session_key}
              session={child}
              session_children={session_children}
            />
          ))}

          {childrenState?.error ? (
            <div className="session-tree__error" role="alert">
              <span>Subagents unavailable: {childrenState.error}</span>
              <button onClick={() => on_children_retry(session.session_key)} type="button">Retry</button>
            </div>
          ) : null}

          {childrenState?.next_cursor ? (
            <button
              className="session-tree__load-more"
              disabled={childrenState.is_loading || childrenState.is_loading_more}
              onClick={() => on_children_load_more(session.session_key)}
              type="button"
            >
              {childrenState.is_loading_more ? "Loading subagents…" : "Load more subagents"}
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

export function Sidebar({
  sessions,
  session_children,
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
  on_children_load,
  on_children_retry,
  on_children_load_more,
  on_retry,
  on_load_more,
}: SidebarProps) {
  const groups = groupSessions(sessions);
  const noProviders = enabled_providers.size === 0;
  const [expandedSessionKeys, setExpandedSessionKeys] = useState<Set<string>>(() => new Set());

  useEffect(() => {
    const ancestors = knownSessionAncestors(sessions, session_children, selected_session_key);
    if (ancestors.length === 0) {
      return;
    }
    setExpandedSessionKeys((current) => {
      const next = new Set(current);
      for (const ancestor of ancestors) {
        next.add(ancestor);
      }
      return next.size === current.size ? current : next;
    });
  }, [selected_session_key, session_children, sessions]);

  const toggleSessionBranch = useCallback((session_key: string) => {
    setExpandedSessionKeys((current) => {
      const next = new Set(current);
      if (next.has(session_key)) {
        next.delete(session_key);
      } else {
        next.add(session_key);
      }
      return next;
    });
  }, []);

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
                <SessionBranch
                  depth={0}
                  expanded_session_keys={expandedSessionKeys}
                  key={session.session_key}
                  on_children_load={on_children_load}
                  on_children_load_more={on_children_load_more}
                  on_children_retry={on_children_retry}
                  on_session_select={on_session_select}
                  on_toggle={toggleSessionBranch}
                  selected_session_key={selected_session_key}
                  session={session}
                  session_children={session_children}
                />
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
