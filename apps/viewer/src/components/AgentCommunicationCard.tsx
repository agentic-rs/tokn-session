import type { EventDetail, EventSummary } from "../lib/types";
import { readableEventContent } from "../lib/state";
import type { TechnicalCardHeading } from "./CardPresentation";
import { MarkdownContent } from "./MarkdownContent";

export function isAgentCommunication(event: EventSummary): boolean {
  return event.type === "agent_activity" && event.agent_activity?.communication != null;
}

export function agentCommunicationHeading(event: EventSummary): TechnicalCardHeading {
  const activity = event.agent_activity;
  const sender = activity?.actor_agent_path?.trim()
    || activity?.actor_session_id?.trim()
    || "Unknown sender";
  const recipient = activity?.target_agent_path?.trim()
    || activity?.target_session_id?.trim()
    || "Unknown recipient";
  const triggerTurn = activity?.communication?.trigger_turn;
  return {
    action: null,
    primary: `Message from ${sender} → ${recipient}`,
    secondary: triggerTurn === true ? "Starts a turn"
      : triggerTurn === false ? "Does not start a turn" : null,
    monospace: false,
  };
}

export function AgentCommunicationCard({
  event,
  detail,
  error,
  is_loading,
  on_retry,
}: {
  event: EventSummary;
  detail: EventDetail | null;
  error: string | null;
  is_loading: boolean;
  on_retry: () => void;
}) {
  if (event.is_hidden || detail?.is_hidden) {
    return <p className="communication-card__notice">Message content is hidden by the provider.</p>;
  }
  const communication = event.agent_activity?.communication;
  const content = readableEventContent(event, detail);
  const needsDetail = communication?.has_text === true;
  const detailMatches = detail?.event_key === event.event_key;
  const detailEvent = detailMatches ? detail?.event : null;
  const detailTruncated = typeof detailEvent === "object" && detailEvent !== null
    && !Array.isArray(detailEvent) && detailEvent.truncated === true;
  return (
    <div className="communication-card">
      {needsDetail && (is_loading || (!detailMatches && !error)) ? (
        <div className="communication-card__state" role="status">
          <span className="inline-spinner" aria-hidden="true" />
          Loading message…
        </div>
      ) : needsDetail && error ? (
        <div className="communication-card__error" role="alert">
          <span>
            <strong>Message unavailable</strong>
            <small>{error}</small>
          </span>
          <button className="text-button" onClick={on_retry} type="button">Try again</button>
        </div>
      ) : content ? (
        content.sections.map((section, index) => (
          <MarkdownContent key={index} content={section.text} />
        ))
      ) : detailTruncated ? (
        <p className="communication-card__notice" role="status">
          Message content exceeds the viewer’s detail size limit.
        </p>
      ) : !communication?.has_encrypted_content ? (
        <p className="communication-card__notice">No readable message body was recorded.</p>
      ) : null}
      {communication?.has_encrypted_content ? (
        <p className="communication-card__notice" role="status">
          <strong>Encrypted message body</strong>
          <span>The encrypted content cannot be read from this session record.</span>
        </p>
      ) : null}
    </div>
  );
}
