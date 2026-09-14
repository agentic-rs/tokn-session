import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { createTranslationEngine, type TranslationEngine } from "../lib/translationEngine";
import type { TranslationStatus } from "../lib/types";

const TranslationContext = createContext<{ engine: TranslationEngine; status: TranslationStatus | null } | null>(null);

/** Check the local translation engine once per viewer without downloading models. */
export function TranslationProvider({ children }: { children: ReactNode }) {
  const [engine] = useState(createTranslationEngine);
  const [status, setStatus] = useState<TranslationStatus | null>(null);
  useEffect(() => {
    let disposed = false;
    void engine.getStatus().then((next) => {
      if (!disposed) setStatus(next);
    }).catch(() => {
      if (!disposed) setStatus({ available: false, reason: `${engine.label} is unavailable in this viewer.` });
    });
    return () => { disposed = true; };
  }, [engine]);
  return <TranslationContext value={{ engine, status }}>{children}</TranslationContext>;
}

export function useTranslationStatus() {
  return useContext(TranslationContext)?.status ?? null;
}

export function useTranslationEngine() {
  return useContext(TranslationContext)?.engine ?? null;
}
