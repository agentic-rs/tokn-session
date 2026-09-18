import { useId, useRef } from "react";
import { useSessionInput } from "../lib/useSessionInput";
import type { SessionSummary } from "../lib/types";

export function SessionComposer({ session, on_accepted }: {
  session: SessionSummary | null;
  on_accepted?: (session_key: string) => void;
}) {
  const input = useSessionInput(session?.session_key ?? null, on_accepted);
  const inputId = useId();
  const textarea = useRef<HTMLTextAreaElement>(null);
  if (!session) return null;

  const disabled = input.checking || !input.availability?.available
    || input.sending || input.delivery_uncertain;
  const overLimit = input.length > input.max_length;

  return (
    <form className="session-composer" aria-label="Send a message to this session" onSubmit={(event) => {
      event.preventDefault();
      void input.send();
    }}>
      <div className="session-composer__content">
        <label htmlFor={inputId}>Message this session</label>
        <textarea
          id={inputId}
          ref={textarea}
          aria-describedby={`${inputId}-status ${inputId}-hint`}
          aria-invalid={overLimit || undefined}
          disabled={disabled}
          onChange={(event) => input.setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)
              && !event.nativeEvent.isComposing && event.keyCode !== 229) {
              event.preventDefault();
              void input.send();
            }
          }}
          placeholder="Write a message…"
          rows={3}
          value={input.draft}
        />
        <div className="session-composer__actions">
          <span id={`${inputId}-hint`} className={overLimit ? "session-composer__limit" : undefined}>
            {overLimit
              ? `${input.length.toLocaleString()} / ${input.max_length.toLocaleString()} characters`
              : "⌘ / Ctrl + Enter to send · Enter for a new line"}
          </span>
          <button className="session-composer__send" disabled={!input.can_send} type="submit">
            {input.sending ? "Sending…" : "Send"}
          </button>
        </div>
        <div className="session-composer__status" id={`${inputId}-status`} role="status">
          {input.notice ? <span>{input.notice}</span> : input.checking
            ? <span>Checking message availability…</span>
            : input.availability?.message ? <span>{input.availability.message}</span> : null}
          {input.delivery_uncertain ? (
            <button className="text-button" onClick={() => {
              input.editMessage();
              requestAnimationFrame(() => textarea.current?.focus());
            }} type="button">Edit message</button>
          ) : null}
          {!input.checking && !input.availability?.available ? (
            <button className="text-button" onClick={input.retryAvailability} type="button">
              Retry availability
            </button>
          ) : null}
        </div>
      </div>
    </form>
  );
}
