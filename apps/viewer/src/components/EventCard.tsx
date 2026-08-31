import { useState } from "react";
import type { EventDetail, EventSummary, ToolCardSummary } from "../lib/types";
import { formatTimestamp } from "../lib/state";
import {
  ChevronIcon,
  ReasoningIcon,
  TechnicalIcon,
  ToolIcon,
  UsageIcon,
  WarningIcon,
} from "./Icons";
import { MarkdownContent } from "./MarkdownContent";
import type { TechnicalCardHeading } from "./CardPresentation";
import { ReasoningCard, reasoningHeading } from "./ReasoningCard";
import { UsageCard, usageHeading } from "./UsageCard";

interface EventCardProps {
  event: EventSummary;
  button_id: string;
  is_selected: boolean;
  is_expanded: boolean;
  detail: EventDetail | null;
  detail_error: string | null;
  detail_loading: boolean;
  on_select: (event_key: string) => void;
  on_toggle: (event_key: string) => void;
  on_retry_detail: () => void;
}

function isMessage(event: EventSummary): boolean {
  return event.type === "message";
}

function usesMarkdown(event: EventSummary): boolean {
  return !event.is_hidden && (event.role === "user" || event.role === "assistant");
}

function humanize(value: string): string {
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

function toolFallbackName(event: EventSummary, tool: ToolCardSummary): string {
  return humanize(tool.tool_name?.trim() || event.title || "Tool call");
}

function toolHeading(event: EventSummary): TechnicalCardHeading {
  const tool = event.tool;
  if (!tool) {
    return {
      action: null,
      primary: event.title || "Tool call",
      secondary: null,
      monospace: false,
    };
  }

  const fallback = toolFallbackName(event, tool);
  switch (tool.kind) {
    case "shell":
      return {
        action: "Shell",
        primary: tool.command?.trim() || fallback,
        secondary: tool.cwd,
        monospace: tool.command !== null,
      };
    case "file_read":
      return {
        action: "Read",
        primary: tool.path?.trim() || fallback,
        secondary: null,
        monospace: tool.path !== null,
      };
    case "file_write":
      return {
        action: "Write",
        primary: tool.path?.trim() || fallback,
        secondary: tool.bytes === null ? null : formatBytes(tool.bytes),
        monospace: tool.path !== null,
      };
    case "file_edit":
      return {
        action: "Edit",
        primary: tool.path?.trim() || fallback,
        secondary: changeSummary(tool),
        monospace: tool.path !== null,
      };
    case "search":
      return {
        action: "Search",
        primary: tool.query?.trim() || fallback,
        secondary: null,
        monospace: false,
      };
    case "web":
      return {
        action: "Web",
        primary: tool.url?.trim() || fallback,
        secondary: null,
        monospace: tool.url !== null,
      };
    case "task":
      return {
        action: "Task",
        primary: tool.task_title?.trim() || fallback,
        secondary: null,
        monospace: false,
      };
    default:
      return {
        action: "Tool",
        primary: fallback,
        secondary: null,
        monospace: false,
      };
  }
}

function changeSummary(tool: ToolCardSummary): string | null {
  const changes = [
    tool.added === null ? null : `+${tool.added}`,
    tool.removed === null ? null : `−${tool.removed}`,
  ].filter((change): change is string => change !== null);
  return changes.length > 0 ? changes.join(" ") : null;
}

function formatBytes(bytes: number): string {
  if (bytes < 1_000) {
    return `${bytes} B`;
  }
  if (bytes < 1_000_000) {
    return `${(bytes / 1_000).toFixed(bytes < 10_000 ? 1 : 0)} KB`;
  }
  return `${(bytes / 1_000_000).toFixed(bytes < 10_000_000 ? 1 : 0)} MB`;
}

function eventIcon(event: EventSummary) {
  if (event.type === "reasoning") {
    return <ReasoningIcon />;
  }
  if (event.type === "usage") {
    return <UsageIcon />;
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
  if (event.type === "error" || event.is_error || (event.tool?.exit_code ?? 0) !== 0) {
    return "error";
  }
  if (event.type === "reasoning") {
    return "reasoning";
  }
  if (event.type === "usage") {
    return "usage";
  }
  if (event.type === "tool_call") {
    return "tool";
  }
  return "technical";
}

function cardHeading(event: EventSummary): TechnicalCardHeading | null {
  if (event.type === "tool_call") {
    return toolHeading(event);
  }
  if (event.type === "usage" && event.usage) {
    return usageHeading(event.usage);
  }
  if (event.type === "reasoning" && event.reasoning) {
    return reasoningHeading(event);
  }
  return null;
}

function usesControlledExpansion(event: EventSummary): boolean {
  return event.type === "tool_call" || event.type === "reasoning";
}

function eventStatus(event: EventSummary): { label: string; tone: string } | null {
  if (event.type !== "tool_call") {
    if (!event.phase || (event.type === "reasoning" && event.phase === "finished")) {
      return null;
    }
    return { label: event.phase, tone: "neutral" };
  }
  if (event.tool?.exit_code !== null
    && event.tool?.exit_code !== undefined
    && event.tool.exit_code !== 0) {
    return { label: `exit ${event.tool.exit_code}`, tone: "error" };
  }
  if (event.is_error) {
    return { label: "failed", tone: "error" };
  }
  if (event.tool?.exit_code !== null && event.tool?.exit_code !== undefined) {
    return { label: `exit ${event.tool.exit_code}`, tone: "success" };
  }
  if (event.phase && event.phase !== "finished") {
    return { label: event.phase === "started" ? "running" : event.phase, tone: "neutral" };
  }
  return null;
}

function ToolOutput({
  detail,
  error,
  event,
  is_loading,
  on_retry,
}: {
  detail: EventDetail | null;
  error: string | null;
  event: EventSummary;
  is_loading: boolean;
  on_retry: () => void;
}) {
  if (event.is_hidden || detail?.is_hidden) {
    return <p className="tool-output__empty">Tool output is hidden by the provider.</p>;
  }
  if (is_loading) {
    return (
      <div className="tool-output__state" role="status">
        <span className="inline-spinner" aria-hidden="true" />
        Loading tool output…
      </div>
    );
  }
  if (error) {
    return (
      <div className="tool-output__error" role="alert">
        <span>
          <strong>Output unavailable</strong>
          <small>{error}</small>
        </span>
        <button className="text-button" onClick={on_retry} type="button">
          Try again
        </button>
      </div>
    );
  }

  const output = detail?.tool_output ?? null;
  if (!output || output.sections.length === 0) {
    const isPending = event.phase !== null && event.phase !== "finished";
    return (
      <p className="tool-output__empty" role="status">
        {isPending ? "Output is not available yet." : "No output was captured for this tool call."}
      </p>
    );
  }

  const toolLabel = toolHeading(event).primary;
  return (
    <div className="tool-output">
      {output.sections.map((section, index) => (
        <section className="tool-output__section" key={`${section.label ?? "output"}-${index}`}>
          {section.label ? <h4>{section.label}</h4> : null}
          <pre
            aria-label={`${toolLabel} ${section.label ? `${section.label} output` : "output"}`}
            data-format={section.format}
            tabIndex={0}
          >
            {section.text}
          </pre>
        </section>
      ))}
      {output.truncated ? (
        <p className="tool-output__notice" role="status">
          Output preview is truncated
          {output.original_size_bytes > 0 ? ` from ${formatBytes(output.original_size_bytes)}` : ""}.
          Inspect the event for the complete bounded detail.
        </p>
      ) : null}
      {output.source_event_key !== event.event_key ? (
        <p className="tool-output__source">Output matched from the related result event.</p>
      ) : null}
    </div>
  );
}

export function EventCard({
  event,
  button_id,
  is_selected,
  is_expanded,
  detail,
  detail_error,
  detail_loading,
  on_select,
  on_toggle,
  on_retry_detail,
}: EventCardProps) {
  const [isLocallyExpanded, setIsLocallyExpanded] = useState(
    event.type === "unknown" || event.type === "error",
  );
  const timestampLabel = formatTimestamp(event.timestamp);

  if (isMessage(event)) {
    const role = event.role ?? "unknown";
    return (
      <article className="message-event" data-role={role} data-selected={is_selected}>
        <div className="message-event__surface">
          <span className="message-event__role">{role}</span>
          {usesMarkdown(event) ? (
            <MarkdownContent
              class_name="message-event__text"
              content={event.summary || event.title}
            />
          ) : (
            <div className="message-event__text message-event__text--plain">
              {event.is_hidden ? "Hidden extension message" : event.summary || event.title}
            </div>
          )}
          <button
            aria-label={`Inspect ${role} message`}
            className="message-event__inspect"
            id={button_id}
            onClick={() => on_select(event.event_key)}
            type="button"
          >
            {event.summary_truncated && !event.is_hidden ? "View full message" : "Inspect"}
          </button>
          <time className="message-event__time" dateTime={event.timestamp ?? undefined} title={timestampLabel}>
            {event.phase && event.phase !== "finished" ? event.phase : ""}
          </time>
        </div>
      </article>
    );
  }

  const heading = cardHeading(event);
  const title = heading?.primary ?? event.title;
  const status = eventStatus(event);
  const regionId = `${button_id}-details`;
  const labelId = `${button_id}-label`;
  const cardIsExpanded = usesControlledExpansion(event) ? is_expanded : isLocallyExpanded;
  return (
    <article
      className="technical-event"
      data-selected={is_selected}
      data-tone={eventTone(event)}
    >
      <div className="technical-event__header">
        <button
          aria-controls={regionId}
          aria-expanded={cardIsExpanded}
          aria-label={heading?.action ? `${heading.action}: ${title}` : title}
          className="technical-event__toggle"
          onClick={() => {
            if (usesControlledExpansion(event)) {
              on_toggle(event.event_key);
            } else {
              setIsLocallyExpanded((expanded) => !expanded);
            }
          }}
          type="button"
        >
          <span className="technical-event__icon">{eventIcon(event)}</span>
          <span className="technical-event__heading">
            <span className="technical-event__title-row">
              {heading?.action ? <span className="tool-action">{heading.action}</span> : null}
              <span
                className="technical-event__title"
                data-monospace={heading?.monospace || undefined}
                id={labelId}
                title={title}
              >
                {title}
              </span>
            </span>
            {heading?.secondary ? (
              <span className="technical-event__secondary" title={heading.secondary}>
                {heading.secondary}
              </span>
            ) : null}
          </span>
          <span className="technical-event__meta">
            {status ? (
              <span className="event-phase" data-tone={status.tone}>{status.label}</span>
            ) : null}
          </span>
          <time dateTime={event.timestamp ?? undefined} title={timestampLabel}>
            {event.timestamp ? timestampLabel : ""}
          </time>
          <ChevronIcon className={cardIsExpanded ? "chevron chevron--open" : "chevron"} />
        </button>
        <button
          aria-label={`Inspect ${title}`}
          className="technical-event__inspect"
          id={button_id}
          onClick={() => on_select(event.event_key)}
          type="button"
        >
          {event.summary_truncated && !event.is_hidden ? "Full detail" : "Inspect"}
        </button>
      </div>
      {cardIsExpanded ? (
        <div
          aria-labelledby={labelId}
          className="technical-event__body"
          id={regionId}
          role="region"
        >
          {event.type === "tool_call" ? (
            <ToolOutput
              detail={detail}
              error={detail_error}
              event={event}
              is_loading={detail_loading}
              on_retry={on_retry_detail}
            />
          ) : event.type === "usage" && event.usage ? (
            <UsageCard usage={event.usage} />
          ) : event.type === "reasoning" ? (
            <ReasoningCard
              detail={detail}
              error={detail_error}
              event={event}
              is_loading={detail_loading}
              on_retry={on_retry_detail}
            />
          ) : (
            <p>{event.is_hidden ? "Hidden extension event" : event.summary || "No summary available."}</p>
          )}
          <span className="event-kind-label">{event.type.replace(/_/g, " ")}</span>
        </div>
      ) : null}
    </article>
  );
}
