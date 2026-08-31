import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listSessions, loadEventDetail, loadEventPage } from "./tauri";
import {
  EVENT_PAGE_SIZE,
  SESSION_PAGE_SIZE,
  errorMessage,
  eventButtonId,
  mergeEvents,
  mergeSessions,
  preserveEventSelection,
  preserveSessionSelection,
} from "./state";
import {
  PROVIDERS,
  type EventDetail,
  type EventSummary,
  type SessionHistoryStatus,
  type SessionSummary,
  type SourceError,
  type ViewerProvider,
} from "./types";

function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timeout = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(timeout);
  }, [delayMs, value]);
  return debounced;
}

const DETAIL_CACHE_LIMIT = 50;

function readCachedDetail(cache: Map<string, EventDetail>, key: string): EventDetail | null {
  const detail = cache.get(key) ?? null;
  if (detail) {
    cache.delete(key);
    cache.set(key, detail);
  }
  return detail;
}

function writeCachedDetail(cache: Map<string, EventDetail>, key: string, detail: EventDetail) {
  cache.delete(key);
  cache.set(key, detail);
  while (cache.size > DETAIL_CACHE_LIMIT) {
    const oldestKey = cache.keys().next().value as string | undefined;
    if (!oldestKey) {
      break;
    }
    cache.delete(oldestKey);
  }
}

function expandedEventNeedsDetail(event: EventSummary | null | undefined): boolean {
  if (!event || event.is_hidden) {
    return false;
  }
  if (event.type === "tool_call") {
    return true;
  }
  return event.type === "reasoning"
    && event.reasoning !== null
    && !event.reasoning.is_redacted
    && (event.reasoning.has_summary || event.reasoning.has_text);
}

export function useViewerState() {
  const [search, setSearchValue] = useState("");
  const debouncedSearch = useDebouncedValue(search.trim(), 180);
  const [enabledProviders, setEnabledProviders] = useState<Set<ViewerProvider>>(
    () => new Set(PROVIDERS),
  );
  const providerKey = PROVIDERS.filter((provider) => enabledProviders.has(provider)).join(",");

  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [selectedSessionKey, setSelectedSessionKey] = useState<string | null>(null);
  const selectedSessionKeyRef = useRef<string | null>(null);
  const [sessionsLoading, setSessionsLoading] = useState(true);
  const [sessionsLoadingMore, setSessionsLoadingMore] = useState(false);
  const [sessionsError, setSessionsError] = useState<string | null>(null);
  const [sourceErrors, setSourceErrors] = useState<SourceError[]>([]);
  const [sessionsCursor, setSessionsCursor] = useState<string | null>(null);
  const [sessionsAttempt, setSessionsAttempt] = useState(0);
  const sessionsRequest = useRef(0);

  const [events, setEvents] = useState<EventSummary[]>([]);
  const [eventsOwnerKey, setEventsOwnerKey] = useState<string | null>(null);
  const eventsOwnerKeyRef = useRef<string | null>(null);
  const [initialPageSessionKey, setInitialPageSessionKey] = useState<string | null>(null);
  const [selectedEventKey, setSelectedEventKey] = useState<string | null>(null);
  const selectedEventKeyRef = useRef<string | null>(null);
  const [eventsLoading, setEventsLoading] = useState(false);
  const [olderLoading, setOlderLoading] = useState(false);
  const [newerLoading, setNewerLoading] = useState(false);
  const [eventsError, setEventsError] = useState<string | null>(null);
  const [olderCursor, setOlderCursor] = useState<string | null>(null);
  const [newerCursor, setNewerCursor] = useState<string | null>(null);
  const [totalEvents, setTotalEvents] = useState<number | null>(null);
  const [historyStatus, setHistoryStatus] = useState<SessionHistoryStatus | null>(null);
  const [eventsAttempt, setEventsAttempt] = useState(0);
  const eventsRequest = useRef(0);

  const [inspectorOpen, setInspectorOpen] = useState(false);
  const inspectorTriggerRef = useRef<HTMLElement | null>(null);
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false);
  const [detail, setDetail] = useState<EventDetail | null>(null);
  const [detailOwnerKey, setDetailOwnerKey] = useState<string | null>(null);
  const detailOwnerKeyRef = useRef<string | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [detailAttempt, setDetailAttempt] = useState(0);
  const detailRequest = useRef(0);
  const detailCache = useRef(new Map<string, EventDetail>());
  const detailLoads = useRef(new Map<string, Promise<EventDetail>>());
  const detailGeneration = useRef(0);
  const [expandedEventKey, setExpandedEventKey] = useState<string | null>(null);
  const [expandedDetail, setExpandedDetail] = useState<EventDetail | null>(null);
  const [expandedDetailOwnerKey, setExpandedDetailOwnerKey] = useState<string | null>(null);
  const [expandedDetailLoading, setExpandedDetailLoading] = useState(false);
  const [expandedDetailError, setExpandedDetailError] = useState<string | null>(null);
  const [expandedDetailAttempt, setExpandedDetailAttempt] = useState(0);
  const expandedDetailRequest = useRef(0);

  const requestDetail = useCallback((sessionKey: string, eventKey: string) => {
    const cacheKey = `${sessionKey}:${eventKey}`;
    const pending = detailLoads.current.get(cacheKey);
    if (pending) {
      return pending;
    }
    const generation = detailGeneration.current;
    let request: Promise<EventDetail>;
    request = loadEventDetail({
      session_key: sessionKey,
      event_key: eventKey,
    }).then((response) => {
      if (detailGeneration.current === generation) {
        writeCachedDetail(detailCache.current, cacheKey, response);
      }
      return response;
    }).finally(() => {
      if (detailLoads.current.get(cacheKey) === request) {
        detailLoads.current.delete(cacheKey);
      }
    });
    detailLoads.current.set(cacheKey, request);
    return request;
  }, []);

  const applyEventSelection = useCallback((eventKey: string | null, openInspector: boolean) => {
    if (selectedEventKeyRef.current !== eventKey) {
      selectedEventKeyRef.current = eventKey;
      detailRequest.current += 1;
      detailOwnerKeyRef.current = null;
      setSelectedEventKey(eventKey);
      setDetailOwnerKey(null);
      setDetail(null);
      setDetailLoading(false);
      setDetailError(null);
    }
    if (!eventKey) {
      inspectorTriggerRef.current = null;
      setInspectorOpen(false);
    } else if (openInspector) {
      setInspectorOpen(true);
    }
  }, []);

  const applySessionSelection = useCallback((sessionKey: string | null) => {
    if (selectedSessionKeyRef.current === sessionKey) {
      return;
    }
    selectedSessionKeyRef.current = sessionKey;
    inspectorTriggerRef.current = null;
    eventsRequest.current += 1;
    eventsOwnerKeyRef.current = null;
    detailGeneration.current += 1;
    detailCache.current.clear();
    detailLoads.current.clear();
    expandedDetailRequest.current += 1;
    setSelectedSessionKey(sessionKey);
    setEventsOwnerKey(null);
    setInitialPageSessionKey(null);
    setEvents([]);
    setEventsLoading(sessionKey !== null);
    setOlderCursor(null);
    setNewerCursor(null);
    setOlderLoading(false);
    setNewerLoading(false);
    setTotalEvents(null);
    setHistoryStatus(null);
    setEventsError(null);
    setExpandedEventKey(null);
    setExpandedDetailOwnerKey(null);
    setExpandedDetail(null);
    setExpandedDetailLoading(false);
    setExpandedDetailError(null);
    applyEventSelection(null, false);
  }, [applyEventSelection]);

  const closeInspector = useCallback(() => {
    const trigger = inspectorTriggerRef.current;
    const fallbackEventKey = selectedEventKeyRef.current;
    detailRequest.current += 1;
    detailOwnerKeyRef.current = null;
    setInspectorOpen(false);
    setDetailOwnerKey(null);
    setDetail(null);
    setDetailLoading(false);
    setDetailError(null);
    window.requestAnimationFrame(() => {
      if (trigger?.isConnected) {
        trigger.focus();
      } else if (fallbackEventKey) {
        document.getElementById(eventButtonId(fallbackEventKey))?.focus();
      }
    });
  }, []);

  useEffect(() => {
    const requestId = ++sessionsRequest.current;
    setSessionsLoadingMore(false);
    if (enabledProviders.size === 0) {
      setSessions([]);
      setSessionsCursor(null);
      setSessionsLoading(false);
      setSessionsError(null);
      setSourceErrors([]);
      applySessionSelection(null);
      return;
    }

    setSessionsLoading(true);
    setSessionsError(null);
    void listSessions({
      query: {
        providers: PROVIDERS.filter((provider) => enabledProviders.has(provider)),
        search: debouncedSearch || undefined,
      },
      limit: SESSION_PAGE_SIZE,
    })
      .then((response) => {
        if (sessionsRequest.current !== requestId) {
          return;
        }
        setSessions(response.sessions);
        setSessionsCursor(response.next_cursor);
        setSourceErrors(response.source_errors);
        applySessionSelection(
          preserveSessionSelection(selectedSessionKeyRef.current, response.sessions),
        );
      })
      .catch((error: unknown) => {
        if (sessionsRequest.current === requestId) {
          setSessions([]);
          setSessionsCursor(null);
          setSourceErrors([]);
          setSessionsError(errorMessage(error));
          applySessionSelection(null);
        }
      })
      .finally(() => {
        if (sessionsRequest.current === requestId) {
          setSessionsLoading(false);
        }
      });
  }, [applySessionSelection, debouncedSearch, enabledProviders, providerKey, sessionsAttempt]);

  useEffect(() => {
    const requestId = ++eventsRequest.current;
    const ownsSession = eventsOwnerKeyRef.current === selectedSessionKey;
    if (!ownsSession) {
      eventsOwnerKeyRef.current = selectedSessionKey;
      setEventsOwnerKey(selectedSessionKey);
      setEvents([]);
      setInitialPageSessionKey(null);
      applyEventSelection(null, false);
    }
    setOlderCursor(null);
    setNewerCursor(null);
    setOlderLoading(false);
    setNewerLoading(false);
    setTotalEvents(null);
    setHistoryStatus(null);
    setEventsError(null);

    if (!selectedSessionKey) {
      setEventsLoading(false);
      return;
    }

    setEventsLoading(true);
    void loadEventPage({
      session_key: selectedSessionKey,
      direction: "backward",
      limit: EVENT_PAGE_SIZE,
    })
      .then((response) => {
        if (eventsRequest.current !== requestId) {
          return;
        }
        setEvents(response.events);
        setOlderCursor(response.previous_cursor);
        setNewerCursor(response.next_cursor);
        setTotalEvents(response.total_events);
        setHistoryStatus(response.history_status);
        setInitialPageSessionKey(selectedSessionKey);
        applyEventSelection(
          preserveEventSelection(selectedEventKeyRef.current, response.events),
          false,
        );
      })
      .catch((error: unknown) => {
        if (eventsRequest.current === requestId) {
          setEventsError(errorMessage(error));
        }
      })
      .finally(() => {
        if (eventsRequest.current === requestId) {
          setEventsLoading(false);
        }
      });
  }, [applyEventSelection, eventsAttempt, selectedSessionKey]);

  useEffect(() => {
    const requestId = ++detailRequest.current;
    setDetail(null);
    setDetailError(null);

    if (!inspectorOpen || !selectedSessionKey || !selectedEventKey) {
      detailOwnerKeyRef.current = null;
      setDetailOwnerKey(null);
      setDetailLoading(false);
      return;
    }

    const cacheKey = `${selectedSessionKey}:${selectedEventKey}`;
    detailOwnerKeyRef.current = cacheKey;
    setDetailOwnerKey(cacheKey);
    const cached = readCachedDetail(detailCache.current, cacheKey);
    if (cached) {
      setDetail(cached);
      setDetailLoading(false);
      return;
    }

    setDetailLoading(true);
    void requestDetail(selectedSessionKey, selectedEventKey)
      .then((response) => {
        if (detailRequest.current !== requestId) {
          return;
        }
        setDetail(response);
      })
      .catch((error: unknown) => {
        if (detailRequest.current === requestId) {
          setDetailError(errorMessage(error));
        }
      })
      .finally(() => {
        if (detailRequest.current === requestId) {
          setDetailLoading(false);
        }
      });
  }, [detailAttempt, inspectorOpen, requestDetail, selectedEventKey, selectedSessionKey]);

  useEffect(() => {
    const requestId = ++expandedDetailRequest.current;
    setExpandedDetail(null);
    setExpandedDetailError(null);

    const expandedEvent = events.find((event) => event.event_key === expandedEventKey);
    if (!selectedSessionKey || !expandedEventKey || !expandedEventNeedsDetail(expandedEvent)) {
      setExpandedDetailOwnerKey(null);
      setExpandedDetailLoading(false);
      return;
    }

    const cacheKey = `${selectedSessionKey}:${expandedEventKey}`;
    setExpandedDetailOwnerKey(cacheKey);
    const cached = readCachedDetail(detailCache.current, cacheKey);
    if (cached) {
      setExpandedDetail(cached);
      setExpandedDetailLoading(false);
      return;
    }

    setExpandedDetailLoading(true);
    void requestDetail(selectedSessionKey, expandedEventKey)
      .then((response) => {
        if (expandedDetailRequest.current === requestId) {
          setExpandedDetail(response);
        }
      })
      .catch((error: unknown) => {
        if (expandedDetailRequest.current === requestId) {
          setExpandedDetailError(errorMessage(error));
        }
      })
      .finally(() => {
        if (expandedDetailRequest.current === requestId) {
          setExpandedDetailLoading(false);
        }
      });
  }, [events, expandedDetailAttempt, expandedEventKey, requestDetail, selectedSessionKey]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") {
        return;
      }
      if (mobileSidebarOpen) {
        setMobileSidebarOpen(false);
      } else if (inspectorOpen) {
        closeInspector();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [closeInspector, inspectorOpen, mobileSidebarOpen]);

  const selectedSession = useMemo(
    () => sessions.find((session) => session.session_key === selectedSessionKey) ?? null,
    [selectedSessionKey, sessions],
  );
  const eventsAreOwned = selectedSessionKey !== null && eventsOwnerKey === selectedSessionKey;
  const visibleEvents = eventsAreOwned ? events : [];
  const selectedEvent = useMemo(
    () => visibleEvents.find((event) => event.event_key === selectedEventKey) ?? null,
    [selectedEventKey, visibleEvents],
  );
  const detailTargetKey = selectedSessionKey && selectedEventKey
    ? `${selectedSessionKey}:${selectedEventKey}`
    : null;
  const detailIsOwned = detailTargetKey !== null && detailOwnerKey === detailTargetKey;
  const expandedVisibleEvent = expandedEventKey === null
    ? null
    : visibleEvents.find((event) => event.event_key === expandedEventKey) ?? null;
  const expandedEventIsVisible = expandedVisibleEvent !== null;
  const expandedDetailTargetKey = selectedSessionKey
    && expandedEventNeedsDetail(expandedVisibleEvent)
    ? `${selectedSessionKey}:${expandedEventKey}`
    : null;
  const expandedDetailIsOwned = expandedDetailTargetKey !== null
    && expandedDetailOwnerKey === expandedDetailTargetKey;

  const toggleProvider = useCallback((provider: ViewerProvider) => {
    sessionsRequest.current += 1;
    setSessionsLoading(true);
    setEnabledProviders((current) => {
      const next = new Set(current);
      if (next.has(provider)) {
        next.delete(provider);
      } else {
        next.add(provider);
      }
      return next;
    });
  }, []);

  const changeSearch = useCallback((value: string) => {
    setSearchValue(value);
  }, []);

  const selectSession = useCallback((sessionKey: string) => {
    applySessionSelection(sessionKey);
    setMobileSidebarOpen(false);
  }, [applySessionSelection]);

  const selectEvent = useCallback((eventKey: string) => {
    inspectorTriggerRef.current = document.getElementById(eventButtonId(eventKey));
    applyEventSelection(eventKey, true);
  }, [applyEventSelection]);

  const toggleEventExpanded = useCallback((eventKey: string) => {
    setExpandedEventKey((current) => current === eventKey ? null : eventKey);
  }, []);

  const toggleInspector = useCallback(() => {
    if (inspectorOpen) {
      closeInspector();
      return;
    }
    if (!selectedEventKeyRef.current) {
      return;
    }
    if (document.activeElement instanceof HTMLElement) {
      inspectorTriggerRef.current = document.activeElement;
    }
    setInspectorOpen(true);
  }, [closeInspector, inspectorOpen]);

  const loadMoreSessions = useCallback(() => {
    if (!sessionsCursor || sessionsLoading || sessionsLoadingMore) {
      return;
    }
    const requestGeneration = sessionsRequest.current;
    setSessionsLoadingMore(true);
    setSessionsError(null);
    void listSessions({
      query: {
        providers: PROVIDERS.filter((provider) => enabledProviders.has(provider)),
        search: debouncedSearch || undefined,
      },
      cursor: sessionsCursor,
      limit: SESSION_PAGE_SIZE,
    })
      .then((response) => {
        if (sessionsRequest.current !== requestGeneration) {
          return;
        }
        setSessions((current) => mergeSessions(current, response.sessions));
        setSessionsCursor(response.next_cursor);
        setSourceErrors(response.source_errors);
      })
      .catch((error: unknown) => {
        if (sessionsRequest.current === requestGeneration) {
          setSessionsError(errorMessage(error));
        }
      })
      .finally(() => {
        if (sessionsRequest.current === requestGeneration) {
          setSessionsLoadingMore(false);
        }
      });
  }, [debouncedSearch, enabledProviders, sessionsCursor, sessionsLoading, sessionsLoadingMore]);

  const loadOlderEvents = useCallback(() => {
    if (
      !selectedSessionKey
      || eventsOwnerKeyRef.current !== selectedSessionKey
      || !olderCursor
      || olderLoading
    ) {
      return;
    }
    const requestGeneration = eventsRequest.current;
    setOlderLoading(true);
    setEventsError(null);
    void loadEventPage({
      session_key: selectedSessionKey,
      cursor: olderCursor,
      direction: "backward",
      limit: EVENT_PAGE_SIZE,
    })
      .then((response) => {
        if (eventsRequest.current !== requestGeneration) {
          return;
        }
        setEvents((current) => mergeEvents(current, response.events, "before"));
        setOlderCursor(response.previous_cursor);
        setTotalEvents(response.total_events);
      })
      .catch((error: unknown) => {
        if (eventsRequest.current === requestGeneration) {
          setEventsError(errorMessage(error));
        }
      })
      .finally(() => {
        if (eventsRequest.current === requestGeneration) {
          setOlderLoading(false);
        }
      });
  }, [olderCursor, olderLoading, selectedSessionKey]);

  const loadNewerEvents = useCallback(() => {
    if (
      !selectedSessionKey
      || eventsOwnerKeyRef.current !== selectedSessionKey
      || !newerCursor
      || newerLoading
    ) {
      return;
    }
    const requestGeneration = eventsRequest.current;
    setNewerLoading(true);
    setEventsError(null);
    void loadEventPage({
      session_key: selectedSessionKey,
      cursor: newerCursor,
      direction: "forward",
      limit: EVENT_PAGE_SIZE,
    })
      .then((response) => {
        if (eventsRequest.current !== requestGeneration) {
          return;
        }
        setEvents((current) => mergeEvents(current, response.events, "after"));
        setNewerCursor(response.next_cursor);
        setTotalEvents(response.total_events);
      })
      .catch((error: unknown) => {
        if (eventsRequest.current === requestGeneration) {
          setEventsError(errorMessage(error));
        }
      })
      .finally(() => {
        if (eventsRequest.current === requestGeneration) {
          setNewerLoading(false);
        }
      });
  }, [newerCursor, newerLoading, selectedSessionKey]);

  return {
    search,
    setSearch: changeSearch,
    enabledProviders,
    toggleProvider,
    sessions,
    selectedSession,
    selectedSessionKey,
    selectSession,
    sessionsLoading,
    sessionsLoadingMore,
    sessionsError,
    sourceErrors,
    sessionsCursor,
    retrySessions: () => setSessionsAttempt((attempt) => attempt + 1),
    loadMoreSessions,
    events: visibleEvents,
    eventsOwnerKey: eventsAreOwned ? eventsOwnerKey : null,
    initialPageLoaded: initialPageSessionKey === selectedSessionKey && eventsAreOwned,
    selectedEvent,
    selectedEventKey: eventsAreOwned ? selectedEventKey : null,
    selectEvent,
    expandedEventKey: expandedEventIsVisible ? expandedEventKey : null,
    toggleEventExpanded,
    expandedDetail: expandedDetailIsOwned ? expandedDetail : null,
    expandedDetailLoading: expandedDetailTargetKey !== null
      && (!expandedDetailIsOwned || expandedDetailLoading),
    expandedDetailError: expandedDetailIsOwned ? expandedDetailError : null,
    retryExpandedDetail: () => setExpandedDetailAttempt((attempt) => attempt + 1),
    eventsLoading: selectedSessionKey !== null && (!eventsAreOwned || eventsLoading),
    olderLoading: eventsAreOwned && olderLoading,
    newerLoading: eventsAreOwned && newerLoading,
    eventsError: eventsAreOwned ? eventsError : null,
    olderCursor: eventsAreOwned ? olderCursor : null,
    newerCursor: eventsAreOwned ? newerCursor : null,
    totalEvents: eventsAreOwned ? totalEvents : null,
    historyStatus: eventsAreOwned ? historyStatus : null,
    retryEvents: () => setEventsAttempt((attempt) => attempt + 1),
    loadOlderEvents,
    loadNewerEvents,
    inspectorOpen,
    closeInspector,
    toggleInspector,
    mobileSidebarOpen,
    setMobileSidebarOpen,
    detail: detailIsOwned ? detail : null,
    detailLoading: inspectorOpen && detailTargetKey !== null && (!detailIsOwned || detailLoading),
    detailError: detailIsOwned ? detailError : null,
    retryDetail: () => setDetailAttempt((attempt) => attempt + 1),
  };
}
