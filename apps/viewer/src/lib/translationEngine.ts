import { createUuid } from "./id";
import { browserTranslationEngine } from "./browserTranslation";
import { cancelTranslation, getTranslationStatus, translateText } from "./tauri";
import { isDesktop } from "./transport";
import type { TranslationStatus } from "./types";

export interface TranslationProgress {
  message: string;
  resume?: () => void;
}

/** A job owns its models and pending work until completion or cancellation. */
export interface TranslationJob {
  translate(texts: string[]): Promise<string[]>;
  /** Idempotent and non-throwing, including during pending model creation. */
  dispose(): void;
}

export interface TranslationEngine {
  label: string;
  description: string;
  getStatus(): Promise<TranslationStatus>;
  start(onProgress: (progress: TranslationProgress) => void): TranslationJob;
}

const appleTranslationEngine: TranslationEngine = {
  label: "Apple Translation",
  description: "Translate to Simplified Chinese with Apple Translation. macOS may ask to download languages.",
  getStatus() { return getTranslationStatus(); },
  start() {
    let disposed = false;
    let request_id: string | null = null;
    return {
      async translate(texts) {
        if (disposed) throw new Error("Translation cancelled.");
        const id = createUuid();
        request_id = id;
        try {
          const response = await translateText({ request_id: id, texts, target_language: "zh-Hans" });
          if (disposed) throw new Error("Translation cancelled.");
          return response.texts;
        } finally {
          if (request_id === id) request_id = null;
        }
      },
      dispose() {
        disposed = true;
        if (request_id) void cancelTranslation(request_id).catch(() => {});
        request_id = null;
      },
    };
  },
};

export function createTranslationEngine(): TranslationEngine {
  return isDesktop() ? appleTranslationEngine : browserTranslationEngine;
}
