import type { TranslationEngine, TranslationJob, TranslationProgress } from "./translationEngine";

type Availability = "unavailable" | "downloadable" | "downloading" | "available";

// These names are the browser's wire contract, not viewer serialized fields.
interface LanguagePair {
  sourceLanguage: string;
  targetLanguage: string;
}

interface CreateOptions {
  signal: AbortSignal;
  monitor(monitor: {
    addEventListener(type: "downloadprogress", listener: (event: { loaded: number }) => void): void;
  }): void;
}

interface NativeResource {
  destroy(): void;
}

interface NativeDetector extends NativeResource {
  detect(text: string, options: { signal: AbortSignal }): Promise<{
    detectedLanguage: string;
    confidence: number;
  }[]>;
}

interface NativeTranslator extends NativeResource {
  translate(text: string, options: { signal: AbortSignal }): Promise<string>;
}

interface BrowserAPIs {
  LanguageDetector: {
    availability(): Promise<Availability>;
    create(options: CreateOptions): Promise<NativeDetector>;
  };
  Translator: {
    availability(options: LanguagePair): Promise<Availability>;
    create(options: LanguagePair & CreateOptions): Promise<NativeTranslator>;
  };
}

// Chrome uses `zh` for Simplified Chinese, and `zh-Hant` for Traditional Chinese.
const TARGET_LANGUAGE = "zh";
const encoder = new TextEncoder();

function browserAPIs(): BrowserAPIs {
  if (!globalThis.isSecureContext) {
    throw new Error("Browser Translation requires HTTPS or localhost.");
  }
  const apis = globalThis as unknown as Partial<BrowserAPIs>;
  if (typeof apis.Translator?.availability !== "function" || typeof apis.Translator.create !== "function"
    || typeof apis.LanguageDetector?.availability !== "function" || typeof apis.LanguageDetector.create !== "function") {
    throw new Error("This browser does not support the Translator and Language Detector APIs.");
  }
  return apis as BrowserAPIs;
}

function cancelled(): DOMException {
  return new DOMException("Translation cancelled.", "AbortError");
}

function destroy(resource: NativeResource) {
  // Aborting a create signal can already have released the native instance.
  try { resource.destroy(); } catch { /* Continue releasing the remaining resources. */ }
}

function isChinese(language: string | null): boolean {
  return language === "zh" || language?.startsWith("zh-") === true;
}

function hasChineseScript(text: string): boolean {
  return /\p{Script=Han}/u.test(text) && !/[\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u.test(text);
}

function otherProse(text: string): string {
  return text.replace(/\p{Script=Han}/gu, " ").trim();
}

function hasOtherPhrase(text: string): boolean {
  // Keep isolated names/acronyms in Chinese prose; a foreign phrase still needs translation.
  return (otherProse(text).match(/[\p{L}\p{M}]{2,}/gu)?.length ?? 0) >= 2;
}

class BrowserTranslationJob implements TranslationJob {
  private readonly controller = new AbortController();
  private readonly resources = new Set<NativeResource>();
  private readonly detector: Promise<NativeDetector>;
  private translator: { source_language: string; instance: NativeTranslator } | null = null;

  constructor(
    private readonly apis: BrowserAPIs,
    private readonly on_progress: (progress: TranslationProgress) => void,
  ) {
    // start() runs inside the original click, before loading full event detail.
    this.detector = this.createResource(
      () => this.apis.LanguageDetector.create(this.createOptions("language detection model")),
      "language detection model",
    );
    // The caller may still be loading detail when this eager creation rejects.
    void this.detector.catch(() => {});
  }

  private checkActive() {
    if (this.controller.signal.aborted) throw cancelled();
  }

  private progress(message: string) {
    if (!this.controller.signal.aborted) this.on_progress({ message });
  }

  /** Also settles promptly if an implementation does not honor its abort signal. */
  private waitFor<T>(promise: Promise<T>): Promise<T> {
    const { signal } = this.controller;
    return new Promise((resolve, reject) => {
      const abort = () => reject(cancelled());
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) abort();
      promise.then((value) => {
        signal.removeEventListener("abort", abort);
        if (signal.aborted) reject(cancelled());
        else resolve(value);
      }, (error: unknown) => {
        signal.removeEventListener("abort", abort);
        reject(error);
      });
    });
  }

  private createOptions(model: string): CreateOptions {
    return {
      signal: this.controller.signal,
      monitor: (monitor) => monitor.addEventListener("downloadprogress", (event) => {
        if (!Number.isFinite(event.loaded)) return;
        const percent = Math.round(Math.max(0, Math.min(1, event.loaded)) * 100);
        this.progress(`Downloading ${model}… ${percent}%`);
      }),
    };
  }

  private own<T extends NativeResource>(promise: Promise<T>): Promise<T> {
    return this.waitFor(promise.then((resource) => {
      if (this.controller.signal.aborted) {
        destroy(resource);
        throw cancelled();
      }
      this.resources.add(resource);
      return resource;
    }));
  }

  private resumeCreation<T extends NativeResource>(create: () => Promise<T>, model: string): Promise<T> {
    this.checkActive();
    const { signal } = this.controller;
    return new Promise((resolve, reject) => {
      let waiting = true;
      const abort = () => {
        waiting = false;
        reject(cancelled());
      };
      signal.addEventListener("abort", abort, { once: true });
      this.on_progress({
        message: `Continue translation to prepare the ${model}.`,
        resume: () => {
          if (!waiting || signal.aborted) return;
          waiting = false;
          signal.removeEventListener("abort", abort);
          this.progress(`Preparing ${model}…`);
          // Invoke create in this click stack: awaiting before it loses activation.
          try { this.own(create()).then(resolve, reject); } catch (error) { reject(error); }
        },
      });
    });
  }

  private async createResource<T extends NativeResource>(
    create: () => Promise<T>,
    model: string,
    availability?: Availability,
  ): Promise<T> {
    this.checkActive();
    if ((availability === "downloadable" || availability === "downloading")
      && globalThis.navigator?.userActivation?.isActive === false) {
      return this.resumeCreation(create, model);
    }
    try {
      return await this.own(create());
    } catch (error) {
      this.checkActive();
      // Retry at most once, and only from another explicit user gesture.
      if (error instanceof DOMException && error.name === "NotAllowedError"
        && globalThis.navigator?.userActivation?.isActive === false) {
        return this.resumeCreation(create, model);
      }
      throw error;
    }
  }

  private async detectLanguage(detector: NativeDetector, text: string, confidence = 0.5): Promise<string | null> {
    this.checkActive();
    const results = await this.waitFor(detector.detect(text, { signal: this.controller.signal }));
    const result = results[0];
    if (!result || result.confidence < confidence || !Number.isFinite(result.confidence)
      || !result.detectedLanguage || result.detectedLanguage === "und") return null;
    try {
      const locale = new Intl.Locale(result.detectedLanguage);
      if (locale.language === "zh") return locale.maximize().script === "Hant" ? "zh-Hant" : "zh";
      return locale.language;
    } catch { return null; }
  }

  private async sourceLanguages(texts: string[]): Promise<(string | null)[]> {
    const detector = await this.detector;
    this.checkActive();
    this.progress("Detecting response language…");
    const context = await this.detectLanguage(detector, texts.join("\n"));
    // Japanese headings can consist entirely of Han characters. Short headings
    // inherit confident Japanese context instead of being misclassified Chinese.
    const useChineseContext = (text: string) => hasChineseScript(text)
      && !(context === "ja" && (text.match(/\p{L}/gu)?.length ?? 0) < 40);
    const chinese_texts = texts.filter(useChineseContext);
    const other_texts = texts.filter((text) => !useChineseContext(text) || hasOtherPhrase(text)).map(otherProse);
    let chinese_language = isChinese(context) ? context : null;
    let other_language = isChinese(context) ? null : context;

    // Group script contexts so a Chinese paragraph cannot hide English headings,
    // and short Markdown fragments inherit a useful source instead of guessing.
    if (chinese_texts.length && !chinese_language) {
      chinese_language = await this.detectLanguage(detector,
        chinese_texts.join("\n").replace(/[^\p{Script=Han}\s]/gu, " "));
    }
    if (other_texts.length && !other_language) {
      other_language = await this.detectLanguage(detector, other_texts.join("\n"));
    }
    const languages: (string | null)[] = [];
    for (const text of texts) {
      const chinese = useChineseContext(text);
      let language = chinese && (!hasOtherPhrase(text) || !other_language)
        ? chinese_language : other_language;
      // Longer paragraphs can reliably override the batch's dominant language.
      if (!chinese && (text.match(/\p{L}/gu)?.length ?? 0) >= 40 && texts.length > 1) {
        language = await this.detectLanguage(detector, text, 0.8) ?? language;
      }
      languages.push(language);
    }
    return languages;
  }

  private async translatorFor(source_language: string): Promise<NativeTranslator> {
    if (this.translator?.source_language === source_language) return this.translator.instance;
    this.checkActive();
    const pair = { sourceLanguage: source_language, targetLanguage: TARGET_LANGUAGE };
    const availability = await this.waitFor(this.apis.Translator.availability(pair));
    if (availability === "unavailable") {
      throw new Error(`This browser cannot translate ${source_language} to Simplified Chinese.`);
    }
    if (this.translator) {
      destroy(this.translator.instance);
      this.resources.delete(this.translator.instance);
      this.translator = null;
    }
    this.progress("Preparing translation model…");
    const instance = await this.createResource(
      () => this.apis.Translator.create({ ...pair, ...this.createOptions("translation model") }),
      "translation model",
      availability,
    );
    this.translator = { source_language, instance };
    return instance;
  }

  private async translateText(translator: NativeTranslator, text: string, source_language: string): Promise<string> {
    const translate = async (prose: string) => {
      this.checkActive();
      const result = await this.waitFor(translator.translate(prose, { signal: this.controller.signal }));
      if (typeof result !== "string" || !result.trim()) {
        throw new Error("The browser translator returned an empty response. Try again.");
      }
      return result;
    };
    if (!hasChineseScript(text) || isChinese(source_language) || source_language === "ja") return translate(text);

    // Preserve Chinese in mixed nodes exactly; only send meaningful foreign
    // phrases to that language's model, with surrounding punctuation retained.
    let translated = "";
    let cursor = 0;
    for (const run of text.matchAll(/[^\p{Script=Han}]+/gu)) {
      if (!hasOtherPhrase(run[0])) continue;
      const parts = run[0].match(/^([^\p{L}]*)([\s\S]*\p{L})([^\p{L}]*)$/u);
      if (!parts) continue;
      const start = run.index! + parts[1].length;
      const end = start + parts[2].length;
      translated += text.slice(cursor, start) + (await translate(parts[2])).trim();
      cursor = end;
    }
    return translated + text.slice(cursor);
  }

  async translate(texts: string[]): Promise<string[]> {
    this.checkActive();
    if (texts.length === 0) return [];
    if (texts.length > 128 || texts.some((text) => encoder.encode(text).length > 16 * 1024)
      || texts.reduce((bytes, text) => bytes + encoder.encode(text).length, 0) > 64 * 1024) {
      throw new Error("This translation batch is too large.");
    }
    const languages = await this.sourceLanguages(texts);
    const groups = new Map<string, number[]>();
    for (let index = 0; index < texts.length; index += 1) {
      const language = languages[index];
      if (!language) throw new Error("The browser could not confidently detect this response's language.");
      if (language !== TARGET_LANGUAGE) groups.set(language, [...(groups.get(language) ?? []), index]);
    }
    const translated = [...texts];
    // One translator lives at a time, even for responses containing many languages.
    for (const [source_language, indexes] of groups) {
      const translator = await this.translatorFor(source_language);
      this.progress("Translating response…");
      for (const index of indexes) {
        this.checkActive();
        translated[index] = await this.translateText(translator, texts[index], source_language);
      }
    }
    this.checkActive();
    return translated;
  }

  dispose() {
    if (this.controller.signal.aborted) return;
    this.controller.abort();
    for (const resource of this.resources) destroy(resource);
    this.resources.clear();
    this.translator = null;
  }
}

export const browserTranslationEngine: TranslationEngine = {
  label: "Browser Translation",
  description: "Translate to Simplified Chinese on this device. The browser may download language models.",
  async getStatus() {
    try {
      const apis = browserAPIs();
      const [detector, translator] = await Promise.all([
        apis.LanguageDetector.availability(),
        apis.Translator.availability({ sourceLanguage: "en", targetLanguage: TARGET_LANGUAGE }),
      ]);
      if (detector === "unavailable") return { available: false, reason: "Language detection is unavailable in this browser." };
      if (translator === "unavailable") return { available: false, reason: "Simplified Chinese translation is unavailable in this browser." };
      return { available: true, reason: null };
    } catch (error) {
      return { available: false, reason: error instanceof Error || error instanceof DOMException
        ? error.message : "Browser Translation is unavailable." };
    }
  },
  start(onProgress) {
    return new BrowserTranslationJob(browserAPIs(), onProgress);
  },
};
