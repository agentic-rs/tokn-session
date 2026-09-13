import type { EventSummary } from "../lib/types";
import { formatTimestamp } from "../lib/state";
import { useMessageTranslation } from "../lib/useMessageTranslation";
import { MarkdownContent } from "./MarkdownContent";
import { useTranslationStatus } from "./TranslationProvider";

interface MessageCardProps {
  event: EventSummary;
  session_key?: string;
  button_id: string;
  is_selected: boolean;
  on_select: (event_key: string) => void;
}

export function MessageCard({ event, session_key, button_id, is_selected, on_select }: MessageCardProps) {
  const status = useTranslationStatus();
  const translation = useMessageTranslation(event, session_key);
  const role = event.role ?? "unknown";
  const presentation = role === "user" ? "bubble" : role === "assistant" ? "transcript" : "technical";
  const usesMarkdown = !event.is_hidden && (role === "user" || role === "assistant");
  const canTranslate = Boolean(status && session_key && role === "assistant" && !event.is_hidden && event.summary.trim());

  return (
    <article className="message-event" data-presentation={presentation} data-role={role} data-selected={is_selected}>
      <div className="message-event__surface">
        <span className="message-event__role">{role}</span>
        {usesMarkdown ? (
          <div aria-busy={translation.loading} lang={translation.showing_translation ? "zh-Hans" : undefined}>
            <MarkdownContent class_name="message-event__text" content={translation.content || event.title} />
          </div>
        ) : (
          <div className="message-event__text message-event__text--plain">
            {event.is_hidden ? "Hidden extension message" : event.summary || event.title}
          </div>
        )}
        {translation.error ? <p className="message-translation__error" role="alert">{translation.error}</p> : null}
        <div className="message-event__actions">
          {canTranslate ? (
            <>
              {translation.loading ? (
                <>
                  <span className="message-translation__status" role="status">
                    <span className="inline-spinner" aria-hidden="true" /> Translating…
                  </span>
                  <button className="message-event__inspect" onClick={translation.cancel} type="button">Cancel</button>
                </>
              ) : translation.translated ? (
                <>
                  <span className="message-translation__status">{translation.showing_translation ? "简体中文 · Apple Translation" : "Original"}</span>
                  <button className="message-event__inspect" onClick={translation.toggle} type="button">
                    {translation.showing_translation ? "Show original" : "Show translation"}
                  </button>
                </>
              ) : (
                <button
                  className="message-event__inspect"
                  disabled={!status?.available}
                  onClick={() => { void translation.translate(); }}
                  title={status?.available ? "Translate to Simplified Chinese with Apple Translation. macOS may ask to download languages." : status?.reason ?? undefined}
                  type="button"
                >
                  {translation.error ? "Retry translation" : "Translate → 简体中文"}
                </button>
              )}
            </>
          ) : null}
          <button
            aria-label={`Inspect ${role} message`}
            className="message-event__inspect"
            id={button_id}
            onClick={() => on_select(event.event_key)}
            type="button"
          >
            {event.summary_truncated && !event.is_hidden ? "View full message" : "Inspect"}
          </button>
        </div>
        <time className="message-event__time" dateTime={event.timestamp ?? undefined} title={formatTimestamp(event.timestamp)}>
          {event.phase && event.phase !== "finished" ? event.phase : ""}
        </time>
      </div>
    </article>
  );
}
