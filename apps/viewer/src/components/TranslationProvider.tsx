import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { getTranslationStatus } from "../lib/tauri";
import { isDesktop } from "../lib/transport";
import type { TranslationStatus } from "../lib/types";

const TranslationContext = createContext<TranslationStatus | null>(null);

/** Check native availability once per viewer; browser clients never invoke it. */
export function TranslationProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<TranslationStatus | null>(null);
  useEffect(() => {
    if (!isDesktop()) return;
    let disposed = false;
    void getTranslationStatus().then((next) => {
      if (!disposed) setStatus(next);
    }).catch(() => {
      if (!disposed) setStatus({ available: false, reason: "Apple Translation is unavailable in this app." });
    });
    return () => { disposed = true; };
  }, []);
  return <TranslationContext value={status}>{children}</TranslationContext>;
}

export function useTranslationStatus() {
  return useContext(TranslationContext);
}
