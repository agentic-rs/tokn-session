import { useEffect, useMemo, useRef, useState } from "react";
import { cancelTranslation, loadEventDetail, translateText } from "./tauri";
import { translateMarkdown } from "./markdownTranslation";
import type { EventSummary } from "./types";

interface TranslationSource {
  session_key: string | undefined;
  event_key: string;
  summary: string;
  summary_truncated: boolean;
}

interface TranslationState {
  source: TranslationSource;
  original: string | null;
  translated: string | null;
  show_translation: boolean;
  loading: boolean;
  error: string | null;
}

/** Results belong to the source text, never just its reusable event index. */
export function useMessageTranslation(event: EventSummary, sessionKey?: string) {
  // A truncated preview cannot identify changes to the rest of a response.
  // Invalidate it whenever the timeline supplies a new source observation.
  const truncatedObservation = event.summary_truncated ? event : null;
  const source = useMemo<TranslationSource>(() => ({
    session_key: sessionKey,
    event_key: event.event_key,
    summary: event.summary,
    summary_truncated: event.summary_truncated,
  }), [sessionKey, event.event_key, event.summary, event.summary_truncated,
    event.is_hidden, event.role, event.phase, truncatedObservation]);
  const currentSource = useRef(source);
  currentSource.current = source;
  const active = useRef<{ source: TranslationSource; request_id: string | null } | null>(null);
  const [state, setState] = useState<TranslationState | null>(null);
  const visible = state?.source === source ? state : null;

  function abortActive() {
    const request = active.current;
    active.current = null;
    if (request?.request_id) void cancelTranslation(request.request_id).catch(() => {});
  }

  useEffect(() => {
    return () => { abortActive(); };
  }, [source]);

  function cancel() {
    abortActive();
    setState((previous) => previous?.source === source ? { ...previous, loading: false } : previous);
  }

  async function translate() {
    if (!sessionKey || active.current || event.is_hidden || event.role !== "assistant" || !event.summary.trim()) return;
    const request = { source, request_id: null as string | null };
    active.current = request;
    const isCurrent = () => active.current === request && currentSource.current === source;
    setState({ source, original: null, translated: null, show_translation: false, loading: true, error: null });
    try {
      const detail = await loadEventDetail({ session_key: sessionKey, event_key: event.event_key });
      if (!isCurrent()) return;
      const value = detail.event;
      if (detail.is_hidden || detail.event_key !== event.event_key
        || !value || typeof value !== "object" || Array.isArray(value)
        || value.type !== "message" || value.role !== "assistant"
        || typeof value.text !== "string" || !value.text.trim()) {
        throw new Error("The full response is unavailable or too large to translate.");
      }
      const original = value.text;
      const expected = source.summary_truncated ? source.summary.replace(/…$/, "") : source.summary;
      if (source.summary_truncated ? !original.startsWith(expected) : original !== expected) {
        throw new Error("This response has changed. Refresh the conversation and try again.");
      }
      setState({ source, original, translated: null, show_translation: false, loading: true, error: null });
      const translated = await translateMarkdown(original, async (texts) => {
        if (!isCurrent()) throw new Error("Translation cancelled.");
        request.request_id = crypto.randomUUID();
        const response = await translateText({
          request_id: request.request_id,
          texts,
          target_language: "zh-Hans",
        });
        if (!isCurrent()) throw new Error("Translation cancelled.");
        request.request_id = null;
        return response.texts;
      });
      if (isCurrent()) setState({ source, original, translated, show_translation: true, loading: false, error: null });
    } catch (error) {
      if (isCurrent()) setState((previous) => ({
        source,
        original: previous?.source === source ? previous.original : null,
        translated: null,
        show_translation: false,
        loading: false,
        error: error instanceof Error ? error.message : String(error),
      }));
    } finally {
      if (active.current === request) active.current = null;
    }
  }

  return {
    content: visible?.show_translation ? visible.translated! : visible?.original ?? event.summary,
    loading: visible?.loading ?? false,
    translated: visible?.translated !== null && visible?.translated !== undefined,
    showing_translation: visible?.show_translation ?? false,
    error: visible?.error ?? null,
    translate,
    cancel,
    toggle: () => setState((previous) => previous?.source === source && previous.translated !== null
      ? { ...previous, show_translation: !previous.show_translation }
      : previous),
  };
}
