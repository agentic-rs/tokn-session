import { useState } from "react";
import type { EventSummary } from "../lib/types";
import { formatTimestamp } from "../lib/state";
import {
  ChevronIcon,
  ReasoningIcon,
  TechnicalIcon,
  ToolIcon,
  WarningIcon,
} from "./Icons";

interface EventCardProps {
  event: EventSummary;
  button_id: string;
  is_selected: boolean;
  on_select: (event_key: string) => void;
}

function isMessage(event: EventSummary): boolean {
  return event.type === "message";
}

function eventIcon(event: EventSummary) {
  if (event.type === "reasoning") {
    return <ReasoningIcon />;
  }
  if (event.type === "tool_call") {
    return <ToolIcon />;
  }
  if (event.type === "unknown" || event.type === "error" || event.is_error) {
    return <WarningIcon />;
  }
  return <TechnicalIcon />;
}

function eventTone(event: EventSummary): string {
  if (event.type === "unknown") {
    return "unknown";
  }
  if (event.type === "error" || event.is_error) {
    return "error";
  }
  if (event.type === "reasoning") {
    return "reasoning";
  }
  if (event.type === "tool_call") {
    return "tool";
  }
  return "technical";
}

export function EventCard({ event, button_id, is_selected, on_select }: EventCardProps) {
  const [isExpanded, setIsExpanded] = useState(event.type === "unknown" || event.type === "error");
  const timestampLabel = formatTimestamp(event.timestamp);

  if (isMessage(event)) {
    const role = event.role ?? "unknown";
    return (
      <article className="message-event" data-role={role} data-selected={is_selected}>
        <button
          aria-label={`Inspect ${role} message`}
          className="message-event__button"
          id={button_id}
          onClick={() => on_select(event.event_key)}
          type="button"
        >
          <span className="message-event__role">{role}</span>
          <span className="message-event__text">
            {event.is_hidden ? "Hidden extension message" : event.summary || event.title}
          </span>
          {event.summary_truncated && !event.is_hidden ? (
            <span className="event-full-content-hint">View full message</span>
          ) : null}
          <time className="message-event__time" dateTime={event.timestamp ?? undefined} title={timestampLabel}>
            {event.phase && event.phase !== "finished" ? event.phase : ""}
          </time>
        </button>
      </article>
    );
  }

  return (
    <article
      className="technical-event"
      data-selected={is_selected}
      data-tone={eventTone(event)}
    >
      <button
        aria-expanded={isExpanded}
        className="technical-event__header"
        id={button_id}
        onClick={() => {
          on_select(event.event_key);
          setIsExpanded((expanded) => !expanded);
        }}
        type="button"
      >
        <span className="technical-event__icon">{eventIcon(event)}</span>
        <span className="technical-event__title">{event.title}</span>
        <span className="technical-event__meta">
          {event.summary_truncated && !event.is_hidden ? (
            <span className="event-full-content-hint">
              {event.type === "reasoning" ? "View full reasoning" : "View full event"}
            </span>
          ) : null}
          {event.phase ? <span className="event-phase">{event.phase}</span> : null}
        </span>
        <time dateTime={event.timestamp ?? undefined} title={timestampLabel}>
          {event.timestamp ? timestampLabel : ""}
        </time>
        <ChevronIcon className={isExpanded ? "chevron chevron--open" : "chevron"} />
      </button>
      {isExpanded ? (
        <div className="technical-event__body">
          <p>{event.is_hidden ? "Hidden extension event" : event.summary || "No summary available."}</p>
          <span className="event-kind-label">{event.type.replace(/_/g, " ")}</span>
        </div>
      ) : null}
    </article>
  );
}
