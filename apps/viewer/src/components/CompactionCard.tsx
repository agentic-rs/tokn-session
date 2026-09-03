import type { EventDetail, EventSummary } from "../lib/types";
import { MarkdownContent } from "./MarkdownContent";
import { readableEventContent } from "../lib/state";

const SCOPE_LABELS: Record<string, string> = {
  context_before: "Context before",
  context_after: "Context after",
  replaced_context: "Replaced context",
};

export function CompactionCard({ event, detail, error, is_loading, on_retry }: {
  event: EventSummary;
  detail: EventDetail | null;
  error: string | null;
  is_loading: boolean;
  on_retry: () => void;
}) {
  const card = event.compaction;
  const summary = readableEventContent(event, detail)?.sections[0]?.text;
  return (
    <div className="compaction-card">
      {card?.trigger ? <p>Trigger: {card.trigger}</p> : null}
      {card?.reason ? <p>{card.reason}</p> : null}
      {card?.measurements.map((item) => (
        <p key={item.scope}>
          {SCOPE_LABELS[item.scope] ?? item.scope}: {item.tokens} tokens
          {item.estimated === true ? " (estimated)" : ""}
        </p>
      ))}
      {is_loading ? <p role="status">Loading compaction summary…</p>
        : error ? (
          <div role="alert">
            <p>Compaction summary unavailable: {error}</p>
            <button className="text-button" onClick={on_retry} type="button">Try again</button>
          </div>
        ) : summary ? <MarkdownContent content={summary} />
          : <p>{card?.summary_opaque
            ? "The provider stored an opaque compaction summary."
            : card?.has_summary ? "Expand to load the compaction summary."
              : "No readable summary was recorded."}</p>}
    </div>
  );
}
