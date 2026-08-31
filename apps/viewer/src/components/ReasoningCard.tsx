import { useState } from "react";
import type { EventDetail, EventSummary } from "../lib/types";
import { readableEventContent } from "../lib/state";
import type { TechnicalCardHeading } from "./CardPresentation";
import { MarkdownContent } from "./MarkdownContent";

interface ReasoningCardProps {
  event: EventSummary;
  detail: EventDetail | null;
  error: string | null;
  is_loading: boolean;
  on_retry: () => void;
}

function opaqueReasoningLabel(event: EventSummary): string {
  if (event.reasoning?.is_redacted) {
    return "Redacted content";
  }
  if (event.reasoning?.has_encrypted_content) {
    return "Encrypted content";
  }
  return "Opaque metadata";
}

export function reasoningHeading(event: EventSummary): TechnicalCardHeading {
  const reasoning = event.reasoning;
  const preview = reasoning?.preview?.trim();
  const hasReadableContent = reasoning?.has_summary || reasoning?.has_text;
  return {
    action: "Reasoning",
    primary: preview || (hasReadableContent ? "Reasoning" : opaqueReasoningLabel(event)),
    secondary: reasoning?.has_summary && reasoning.has_text
      ? "Summary · detailed reasoning available"
      : null,
    monospace: false,
  };
}

function ReasoningSection({
  label,
  text,
}: {
  label: string | null;
  text: string;
}) {
  return (
    <section className="reasoning-card__section">
      {label ? <h4>{label}</h4> : null}
      <MarkdownContent content={text} />
    </section>
  );
}

export function ReasoningCard({
  event,
  detail,
  error,
  is_loading,
  on_retry,
}: ReasoningCardProps) {
  const [showDetailedReasoning, setShowDetailedReasoning] = useState(false);
  const reasoning = event.reasoning;
  if (event.is_hidden || detail?.is_hidden) {
    return <p className="reasoning-card__notice">Reasoning is hidden by the provider.</p>;
  }
  if (reasoning?.is_redacted) {
    return <p className="reasoning-card__notice">Reasoning content is redacted by the provider.</p>;
  }
  if (!reasoning?.has_summary && !reasoning?.has_text) {
    return (
      <p className="reasoning-card__notice">
        {reasoning?.has_encrypted_content
          ? "Reasoning content is encrypted and cannot be shown inline."
          : "This reasoning record has no readable text."}
      </p>
    );
  }
  if (is_loading) {
    return (
      <div className="reasoning-card__state" role="status">
        <span className="inline-spinner" aria-hidden="true" />
        Loading reasoning…
      </div>
    );
  }
  if (error) {
    return (
      <div className="reasoning-card__error" role="alert">
        <span>
          <strong>Reasoning unavailable</strong>
          <small>{error}</small>
        </span>
        <button className="text-button" onClick={on_retry} type="button">
          Try again
        </button>
      </div>
    );
  }

  const content = readableEventContent(event, detail);
  if (!content) {
    return <p className="reasoning-card__notice">No readable reasoning was captured for this event.</p>;
  }
  const [firstSection, ...remainingSections] = content.sections;
  if (!firstSection) {
    return <p className="reasoning-card__notice">No readable reasoning was captured for this event.</p>;
  }

  return (
    <div className="reasoning-card">
      <ReasoningSection
        label={remainingSections.length > 0 ? firstSection.label : null}
        text={firstSection.text}
      />
      {remainingSections.length > 0 ? (
        <details className="reasoning-card__details" open={showDetailedReasoning}>
          <summary
            onClick={(event) => {
              event.preventDefault();
              setShowDetailedReasoning((isOpen) => !isOpen);
            }}
          >
            Detailed reasoning
          </summary>
          {remainingSections.map((section, index) => (
            <ReasoningSection
              key={`${section.label ?? "reasoning"}-${index}`}
              label={section.label}
              text={section.text}
            />
          ))}
        </details>
      ) : null}
    </div>
  );
}
