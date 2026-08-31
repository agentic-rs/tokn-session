import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { EventDetail, EventSummary } from "../lib/types";
import { formatTimestamp, providerLabel, readableEventContent } from "../lib/state";
import { CloseIcon, WarningIcon } from "./Icons";
import { MarkdownContent } from "./MarkdownContent";

interface InspectorProps {
  is_open: boolean;
  event: EventSummary | null;
  detail: EventDetail | null;
  is_loading: boolean;
  error: string | null;
  on_close: () => void;
  on_retry: () => void;
}

type InspectorTab = "content" | "normalized" | "native";

export function Inspector({
  is_open,
  event,
  detail,
  is_loading,
  error,
  on_close,
  on_retry,
}: InspectorProps) {
  const [activeTab, setActiveTab] = useState<InspectorTab>("normalized");
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const userSelectedTabRef = useRef(false);

  useLayoutEffect(() => {
    userSelectedTabRef.current = false;
    setActiveTab("normalized");
  }, [event?.event_key]);

  useEffect(() => {
    if (!is_open) {
      return;
    }
    const frame = window.requestAnimationFrame(() => closeButtonRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [event?.event_key, is_open]);

  const readableContent = useMemo(
    () => event ? readableEventContent(event, detail) : null,
    [detail, event],
  );
  const isRedactedReasoning = event?.type === "reasoning" && event.reasoning?.is_redacted === true;
  const contentAvailable = readableContent !== null;
  useLayoutEffect(() => {
    if (contentAvailable && !userSelectedTabRef.current) {
      setActiveTab("content");
    } else if (!contentAvailable && activeTab === "content") {
      setActiveTab("normalized");
    }
  }, [activeTab, contentAvailable]);

  const redactedNormalizedEvent = isRedactedReasoning && event ? {
    type: "reasoning",
    provider: event.provider,
    redacted: true,
  } : null;
  const visibleValue = activeTab === "native"
    ? isRedactedReasoning ? null : detail?.native
    : activeTab === "normalized"
      ? redactedNormalizedEvent ?? detail?.event
      : undefined;
  const formattedValue = useMemo(() => {
    if (visibleValue === undefined) {
      return "";
    }
    return JSON.stringify(visibleValue, null, 2);
  }, [visibleValue]);
  const nativeAvailable = detail !== null && detail.native !== null && !detail.is_hidden && !isRedactedReasoning;

  function selectTab(tab: InspectorTab) {
    userSelectedTabRef.current = true;
    setActiveTab(tab);
  }

  function onTabKeyDown(keyboardEvent: KeyboardEvent<HTMLButtonElement>) {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(keyboardEvent.key)) {
      return;
    }
    keyboardEvent.preventDefault();
    const availableTabs: InspectorTab[] = [
      ...(contentAvailable ? ["content" as const] : []),
      "normalized",
      ...(nativeAvailable ? ["native" as const] : []),
    ];
    const currentIndex = availableTabs.indexOf(activeTab);
    const nextIndex = keyboardEvent.key === "Home"
      ? 0
      : keyboardEvent.key === "End"
        ? availableTabs.length - 1
        : keyboardEvent.key === "ArrowRight"
          ? (currentIndex + 1) % availableTabs.length
          : (currentIndex - 1 + availableTabs.length) % availableTabs.length;
    const nextTab = availableTabs[nextIndex]!;
    selectTab(nextTab);
    window.requestAnimationFrame(() => document.getElementById(`inspector-tab-${nextTab}`)?.focus());
  }

  return (
    <aside
      aria-label="Event inspector"
      aria-hidden={!is_open}
      className="inspector"
      data-open={is_open}
      inert={!is_open}
    >
      <header className="inspector__header" data-tauri-drag-region>
        <div>
          <p className="eyebrow">INSPECTOR</p>
          <h2>{event?.title ?? "Event detail"}</h2>
        </div>
        <button
          aria-label="Close inspector"
          className="icon-button"
          onClick={on_close}
          ref={closeButtonRef}
          type="button"
        >
          <CloseIcon />
        </button>
      </header>

      {!event ? (
        <div className="inspector__empty">
          <span className="empty-orbit" aria-hidden="true" />
          <strong>No event selected</strong>
          <p>Select any message or technical event to inspect its normalized shape.</p>
        </div>
      ) : (
        <>
          <dl className="event-facts">
            <div>
              <dt>Provider</dt>
              <dd>
                <span className="provider-badge" data-provider={event.provider}>
                  {providerLabel(event.provider)}
                </span>
              </dd>
            </div>
            <div>
              <dt>Event</dt>
              <dd>{event.type.replace(/_/g, " ")}</dd>
            </div>
            <div>
              <dt>Time</dt>
              <dd>{formatTimestamp(event.timestamp)}</dd>
            </div>
            {event.phase ? (
              <div>
                <dt>Phase</dt>
                <dd>{event.phase}</dd>
              </div>
            ) : null}
          </dl>

          {detail?.is_hidden || event.is_hidden ? (
            <div className="redaction-notice" role="status">
              <WarningIcon />
              <span>
                <strong>Content hidden by provider</strong>
                <small>The viewer does not request or render its native payload.</small>
              </span>
            </div>
          ) : null}

          {isRedactedReasoning ? (
            <div className="redaction-notice" role="status">
              <WarningIcon />
              <span>
                <strong>Reasoning redacted by provider</strong>
                <small>The viewer does not render its readable or native payload.</small>
              </span>
            </div>
          ) : null}

          {event.summary_truncated
            && detail
            && !readableContent
            && !detail.is_hidden
            && !event.is_hidden
            && !isRedactedReasoning ? (
              <div className="content-notice" role="status">
                The timeline preview was capped, but this event has no readable plain-text field.
                Normalized and native data remain available below.
              </div>
            ) : null}

          <div className="inspector-tabs" role="tablist" aria-label="Event representation">
            {contentAvailable ? (
              <button
                aria-controls="inspector-event-panel"
                aria-selected={activeTab === "content"}
                className="inspector-tab"
                id="inspector-tab-content"
                onKeyDown={onTabKeyDown}
                onClick={() => selectTab("content")}
                role="tab"
                tabIndex={activeTab === "content" ? 0 : -1}
                type="button"
              >
                Content
              </button>
            ) : null}
            <button
              aria-controls="inspector-event-panel"
              aria-selected={activeTab === "normalized"}
              className="inspector-tab"
              id="inspector-tab-normalized"
              onKeyDown={onTabKeyDown}
              onClick={() => selectTab("normalized")}
              role="tab"
              tabIndex={activeTab === "normalized" ? 0 : -1}
              type="button"
            >
              Normalized
            </button>
            <button
              aria-controls="inspector-event-panel"
              aria-selected={activeTab === "native"}
              className="inspector-tab"
              disabled={!nativeAvailable}
              id="inspector-tab-native"
              onKeyDown={onTabKeyDown}
              onClick={() => selectTab("native")}
              role="tab"
              tabIndex={activeTab === "native" ? 0 : -1}
              type="button"
            >
              Native
            </button>
          </div>

          <div
            aria-labelledby={`inspector-tab-${activeTab}`}
            className="inspector__content"
            id="inspector-event-panel"
            role="tabpanel"
          >
            {is_loading ? (
              <div className="detail-loading" role="status">
                <span className="spinner" />
                Loading event detail…
              </div>
            ) : null}
            {!is_loading && error ? (
              <div className="detail-error" role="alert">
                <strong>Detail unavailable</strong>
                <span>{error}</span>
                <button className="text-button" onClick={on_retry} type="button">
                  Try again
                </button>
              </div>
            ) : null}
            {!is_loading && !error && detail ? (
              activeTab === "content" && readableContent ? (
                <div className="readable-content">
                  {readableContent.sections.map((section, index) => (
                    <section key={`${section.label ?? "content"}-${index}`}>
                      {section.label ? <h3>{section.label}</h3> : null}
                      <MarkdownContent
                        class_name="readable-content__text"
                        content={section.text}
                      />
                    </section>
                  ))}
                </div>
              ) : visibleValue === null ? (
                <div className="detail-empty">No native payload is available for this event.</div>
              ) : visibleValue !== undefined ? (
                <pre>{formattedValue}</pre>
              ) : null
            ) : null}
          </div>
        </>
      )}
    </aside>
  );
}
