import { useLayoutEffect, useRef } from "react";
import type {
  EventDetail,
  EventSummary,
  SessionHistoryStatus,
  SessionSummary,
} from "../lib/types";
import {
  eventButtonId,
  formatTimestamp,
  providerLabel,
  sessionDisplayTitle,
  shortSessionId,
  subagentDetail,
} from "../lib/state";
import { InspectorIcon, PanelIcon } from "./Icons";
import { EventCard } from "./EventCard";
import { LoadingRows, StateView } from "./StateView";

interface ConversationProps {
  session: SessionSummary | null;
  events: EventSummary[];
  selected_event_key: string | null;
  expanded_event_key: string | null;
  expanded_detail: EventDetail | null;
  expanded_detail_error: string | null;
  expanded_detail_loading: boolean;
  initial_page_loaded: boolean;
  is_loading: boolean;
  is_loading_older: boolean;
  is_loading_newer: boolean;
  error: string | null;
  has_older: boolean;
  has_newer: boolean;
  total_events: number | null;
  history_status: SessionHistoryStatus | null;
  inspector_open: boolean;
  on_sidebar_open: () => void;
  on_inspector_toggle: () => void;
  on_event_select: (event_key: string) => void;
  on_event_toggle: (event_key: string) => void;
  on_load_older: () => void;
  on_load_newer: () => void;
  on_retry: () => void;
  on_retry_expanded_detail: () => void;
}

export function Conversation({
  session,
  events,
  selected_event_key,
  expanded_event_key,
  expanded_detail,
  expanded_detail_error,
  expanded_detail_loading,
  initial_page_loaded,
  is_loading,
  is_loading_older,
  is_loading_newer,
  error,
  has_older,
  has_newer,
  total_events,
  history_status,
  inspector_open,
  on_sidebar_open,
  on_inspector_toggle,
  on_event_select,
  on_event_toggle,
  on_load_older,
  on_load_newer,
  on_retry,
  on_retry_expanded_detail,
}: ConversationProps) {
  const timelineRef = useRef<HTMLDivElement>(null);
  const priorScrollHeight = useRef<number | null>(null);
  const observedSessionKey = useRef<string | null>(null);
  const didInitialScroll = useRef(false);

  useLayoutEffect(() => {
    const timeline = timelineRef.current;
    if (!timeline) {
      return;
    }
    const sessionKey = session?.session_key ?? null;
    if (observedSessionKey.current !== sessionKey) {
      observedSessionKey.current = sessionKey;
      didInitialScroll.current = false;
      priorScrollHeight.current = null;
    }
    if (sessionKey && initial_page_loaded && !didInitialScroll.current) {
      timeline.scrollTop = timeline.scrollHeight;
      didInitialScroll.current = true;
      return;
    }
    if (!is_loading_older && priorScrollHeight.current !== null) {
      timeline.scrollTop += timeline.scrollHeight - priorScrollHeight.current;
      priorScrollHeight.current = null;
    }
  }, [events, initial_page_loaded, is_loading_older, session?.session_key]);

  function loadOlder() {
    priorScrollHeight.current = timelineRef.current?.scrollHeight ?? null;
    on_load_older();
  }

  const knownCount = total_events ?? session?.event_count ?? session?.message_count ?? null;
  const childDetail = session?.is_subagent ? subagentDetail(session) : null;
  const countLabel = total_events !== null || session?.event_count !== null
    ? knownCount !== null
      ? `${knownCount} events`
      : "Event count unavailable"
    : session?.message_count !== null
      ? `${knownCount} messages`
      : is_loading
        ? "Loading events…"
        : "Event count unavailable";

  return (
    <main className="conversation">
      <header className="conversation__header" data-tauri-drag-region>
        <button
          aria-label="Open sessions"
          className="icon-button mobile-sidebar-button"
          onClick={on_sidebar_open}
          type="button"
        >
          <PanelIcon />
        </button>
        {session ? (
          <div className="conversation__identity">
            <div className="conversation__title-row">
              <h2 title={sessionDisplayTitle(session)}>{sessionDisplayTitle(session)}</h2>
              <span className="provider-badge" data-provider={session.provider}>
                {providerLabel(session.provider)}
              </span>
            </div>
            <p title={`${session.cwd ?? "Unassigned project"}\nSession ${session.session_id}`}>
              {session.is_subagent ? (
                <>
                  <span title={childDetail ?? undefined}>
                    {childDetail ? `Subagent · ${childDetail}` : "Subagent"}
                  </span>
                  <span aria-hidden="true"> · </span>
                </>
              ) : null}
              {session.project ?? session.cwd ?? "Unassigned project"}
              <span aria-hidden="true"> · </span>
              <span
                aria-label={`Session ${session.session_id}`}
                className="conversation__session-id"
              >
                {shortSessionId(session.session_id)}
              </span>
              <span aria-hidden="true"> · </span>
              {countLabel}
            </p>
          </div>
        ) : (
          <div className="conversation__identity conversation__identity--empty">
            <h2>Session viewer</h2>
            <p>Read-only, across every known provider</p>
          </div>
        )}
        <button
          aria-label={inspector_open ? "Close event inspector" : "Open event inspector"}
          aria-pressed={inspector_open}
          className="icon-button inspector-toggle"
          disabled={!selected_event_key}
          onClick={on_inspector_toggle}
          title="Event inspector"
          type="button"
        >
          <InspectorIcon />
        </button>
      </header>

      <div className="conversation__timeline" ref={timelineRef}>
        {!session ? (
          <StateView
            message="Choose a session from the sidebar to inspect its normalized conversation."
            title="Select a session"
          />
        ) : null}

        {session && is_loading && events.length === 0 ? (
          <div className="timeline-loading">
            <LoadingRows count={5} />
          </div>
        ) : null}

        {session && !is_loading && error && events.length === 0 ? (
          <StateView
            action_label="Try again"
            message={error}
            on_action={on_retry}
            title="Conversation unavailable"
            tone="error"
          />
        ) : null}

        {session && !is_loading && !error && events.length === 0 ? (
          <StateView
            message="The provider returned a valid session with no normalized events."
            title="No events in this session"
          />
        ) : null}

        {session && events.length > 0 ? (
          <div className="timeline" aria-label="Session event timeline">
            {history_status && history_status !== "complete" ? (
              <div className="history-notice" role="status">
                This provider exposes only part of the subagent history.
              </div>
            ) : null}
            {error ? (
              <div className="pagination-error" role="alert">
                <span>{error}</span>
                <button className="text-button" onClick={on_retry} type="button">
                  Reload
                </button>
              </div>
            ) : null}
            {has_older ? (
              <button
                className="page-button"
                disabled={is_loading_older}
                onClick={loadOlder}
                type="button"
              >
                {is_loading_older ? "Loading earlier events…" : "Load earlier events"}
              </button>
            ) : (
              <div className="timeline-boundary">
                <span />
                <span>Session start</span>
                <span />
              </div>
            )}

            {events.map((event) => (
              <EventCard
                button_id={eventButtonId(event.event_key)}
                event={event}
                detail={event.event_key === expanded_event_key ? expanded_detail : null}
                detail_error={event.event_key === expanded_event_key ? expanded_detail_error : null}
                detail_loading={
                  event.event_key === expanded_event_key && expanded_detail_loading
                }
                is_expanded={event.event_key === expanded_event_key}
                is_selected={event.event_key === selected_event_key}
                key={`${session.session_key}:${event.event_key}`}
                on_select={on_event_select}
                on_toggle={on_event_toggle}
                on_retry_detail={on_retry_expanded_detail}
              />
            ))}

            {has_newer ? (
              <button
                className="page-button"
                disabled={is_loading_newer}
                onClick={on_load_newer}
                type="button"
              >
                {is_loading_newer ? "Loading newer events…" : "Load newer events"}
              </button>
            ) : (
              <div className="timeline-boundary timeline-boundary--end">
                <span />
                <span>{formatTimestamp(events[events.length - 1]?.timestamp ?? null)}</span>
                <span />
              </div>
            )}
          </div>
        ) : null}
      </div>
    </main>
  );
}
