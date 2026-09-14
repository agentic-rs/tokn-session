import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { browserTranslationEngine } from "./browserTranslation";
import type { TranslationJob, TranslationProgress } from "./translationEngine";

interface Detection {
  detectedLanguage: string;
  confidence: number;
}

interface CreateOptions {
  signal: AbortSignal;
  monitor(monitor: {
    addEventListener(type: "downloadprogress", callback: (event: { loaded: number }) => void): void;
  }): void;
}

interface Pair {
  sourceLanguage: string;
  targetLanguage: string;
}

type Availability = "available" | "unavailable" | "downloadable" | "downloading";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

const detection = (language: string, confidence = 0.99): Detection[] => [{ detectedLanguage: language, confidence }];
const activation = { isActive: true };
const detector = {
  detect: vi.fn<(text: string, options: { signal: AbortSignal }) => Promise<Detection[]>>(),
  destroy: vi.fn(),
};
const translator = {
  translate: vi.fn<(text: string, options: { signal: AbortSignal }) => Promise<string>>(),
  destroy: vi.fn(),
};
const apis = {
  LanguageDetector: {
    availability: vi.fn<() => Promise<Availability>>(),
    create: vi.fn<(options: CreateOptions) => Promise<typeof detector>>(),
  },
  Translator: {
    availability: vi.fn<(options: Pair) => Promise<Availability>>(),
    create: vi.fn<(options: Pair & CreateOptions) => Promise<typeof translator>>(),
  },
};
const jobs: TranslationJob[] = [];

function start(onProgress: (progress: TranslationProgress) => void = vi.fn()) {
  const job = browserTranslationEngine.start(onProgress);
  jobs.push(job);
  return job;
}

beforeEach(() => {
  vi.resetAllMocks();
  activation.isActive = true;
  vi.stubGlobal("isSecureContext", true);
  vi.stubGlobal("navigator", { userActivation: activation });
  vi.stubGlobal("LanguageDetector", apis.LanguageDetector);
  vi.stubGlobal("Translator", apis.Translator);
  detector.detect.mockResolvedValue(detection("en"));
  translator.translate.mockImplementation(async (text) => `译文 ${text}`);
  apis.LanguageDetector.availability.mockResolvedValue("available");
  apis.LanguageDetector.create.mockResolvedValue(detector);
  apis.Translator.availability.mockResolvedValue("available");
  apis.Translator.create.mockResolvedValue(translator);
});

afterEach(() => {
  jobs.splice(0).forEach((job) => job.dispose());
  vi.unstubAllGlobals();
});

describe("browser translation availability", () => {
  it("requires a secure context without creating or probing models", async () => {
    vi.stubGlobal("isSecureContext", false);
    expect(await browserTranslationEngine.getStatus()).toEqual({
      available: false, reason: "Browser Translation requires HTTPS or localhost.",
    });
    expect(apis.LanguageDetector.availability).not.toHaveBeenCalled();
    expect(apis.Translator.create).not.toHaveBeenCalled();
    expect(() => start()).toThrow("HTTPS or localhost");
  });

  it.each(["Translator", "LanguageDetector"])("requires the %s API", async (api) => {
    vi.stubGlobal(api, undefined);
    expect(await browserTranslationEngine.getStatus()).toEqual({
      available: false, reason: "This browser does not support the Translator and Language Detector APIs.",
    });
  });

  it.each<Availability>(["available", "downloadable", "downloading"])("allows %s models without downloading them", async (availability) => {
    apis.LanguageDetector.availability.mockResolvedValue(availability);
    apis.Translator.availability.mockResolvedValue(availability);
    expect(await browserTranslationEngine.getStatus()).toEqual({ available: true, reason: null });
    expect(apis.Translator.availability).toHaveBeenCalledWith({ sourceLanguage: "en", targetLanguage: "zh" });
    expect(apis.LanguageDetector.create).not.toHaveBeenCalled();
    expect(apis.Translator.create).not.toHaveBeenCalled();
  });

  it("reports unavailable detection and translation models", async () => {
    apis.LanguageDetector.availability.mockResolvedValue("unavailable");
    expect(await browserTranslationEngine.getStatus()).toEqual({
      available: false, reason: "Language detection is unavailable in this browser.",
    });
    apis.LanguageDetector.availability.mockResolvedValue("available");
    apis.Translator.availability.mockResolvedValue("unavailable");
    expect(await browserTranslationEngine.getStatus()).toEqual({
      available: false, reason: "Simplified Chinese translation is unavailable in this browser.",
    });
  });

  it("makes policy failures readable", async () => {
    apis.Translator.availability.mockRejectedValue(new DOMException("Translation blocked by browser policy", "NotAllowedError"));
    expect(await browserTranslationEngine.getStatus()).toEqual({
      available: false, reason: "Translation blocked by browser policy",
    });
  });
});

describe("browser language detection and translation", () => {
  it("starts detection in the original click and gives short Markdown fragments the batch context", async () => {
    const job = start();
    expect(apis.LanguageDetector.create).toHaveBeenCalledTimes(1);
    expect(detector.detect).not.toHaveBeenCalled();
    const texts = ["Create a", "todo", "application"];
    expect(await job.translate(texts)).toEqual(texts.map((text) => `译文 ${text}`));
    expect(detector.detect).toHaveBeenCalledTimes(1);
    expect(detector.detect).toHaveBeenCalledWith(texts.join("\n"), { signal: expect.any(AbortSignal) });
    expect(apis.Translator.create).toHaveBeenCalledWith(expect.objectContaining({ sourceLanguage: "en", targetLanguage: "zh" }));
    await job.translate(["The next paragraph"]);
    expect(apis.Translator.create).toHaveBeenCalledTimes(1);
    job.dispose();
    job.dispose();
    expect(detector.destroy).toHaveBeenCalledTimes(1);
    expect(translator.destroy).toHaveBeenCalledTimes(1);
  });

  it.each(["zh", "zh-Hans", "zh-CN"])("preserves already simplified Chinese detected as %s", async (language) => {
    detector.detect.mockResolvedValue(detection(language));
    expect(await start().translate(["这是一段简体中文。"])).toEqual(["这是一段简体中文。"]);
    expect(apis.Translator.create).not.toHaveBeenCalled();
  });

  it("translates Traditional Chinese to Simplified Chinese", async () => {
    detector.detect.mockResolvedValue(detection("zh-TW"));
    await start().translate(["這是一段繁體中文。"]);
    expect(apis.Translator.create).toHaveBeenCalledWith(expect.objectContaining({ sourceLanguage: "zh-Hant", targetLanguage: "zh" }));
  });

  it("translates English headings in Chinese-dominant batches", async () => {
    detector.detect.mockImplementation(async (text) => detection(/\p{Script=Han}/u.test(text) ? "zh" : "en"));
    const texts = ["这是一段中文说明，它不应隐藏英文标题。", "Installation", "Follow these steps"];
    expect(await start().translate(texts)).toEqual([texts[0], "译文 Installation", "译文 Follow these steps"]);
    expect(translator.translate.mock.calls.map(([text]) => text)).toEqual(texts.slice(1));
  });

  it("translates a foreign phrase embedded in Chinese prose while preserving isolated acronyms", async () => {
    detector.detect.mockImplementation(async (text) => detection(/\p{Script=Han}/u.test(text) ? "zh" : "en"));
    const texts = ["这是 API 文档。", "请 follow the instructions to finish setup，谢谢。"];
    expect(await start().translate(texts)).toEqual([texts[0], "请 译文 follow the instructions to finish setup，谢谢。"]);
    expect(translator.translate).toHaveBeenCalledTimes(1);
    expect(translator.translate.mock.calls[0][0]).toBe("follow the instructions to finish setup");
  });

  it("lets short Kanji headings inherit the surrounding Japanese language", async () => {
    const paragraph = "この説明を読んで、アプリケーションの使い方を確認してください。";
    detector.detect.mockImplementation(async (text) => detection(text.includes("アプリ") ? "ja" : "zh"));
    const texts = ["詳細", paragraph, "結果"];
    expect(await start().translate(texts)).toEqual(texts.map((text) => `译文 ${text}`));
    expect(apis.Translator.create.mock.calls[0][0].sourceLanguage).toBe("ja");
  });

  it("preserves Chinese fragments in English-dominant batches", async () => {
    detector.detect.mockImplementation(async (text) => detection(/[a-z]/i.test(text) ? "en" : "zh"));
    const texts = ["Follow these instructions to finish", "中文说明"];
    expect(await start().translate(texts)).toEqual([`译文 ${texts[0]}`, texts[1]]);
  });

  it("recognizes a long paragraph in another language and releases the previous translator", async () => {
    const english = "Follow the installation instructions below to prepare the application for use.";
    const french = "Veuillez suivre les instructions pour installer cette application sur votre ordinateur.";
    const second = { translate: vi.fn(async () => "第二段译文"), destroy: vi.fn() };
    detector.detect.mockImplementation(async (text) => detection(text === french ? "fr" : "en"));
    apis.Translator.create.mockResolvedValueOnce(translator).mockResolvedValueOnce(second);
    const job = start();
    expect(await job.translate([english, french])).toEqual([`译文 ${english}`, "第二段译文"]);
    expect(apis.Translator.create.mock.calls.map(([options]) => options.sourceLanguage)).toEqual(["en", "fr"]);
    expect(translator.destroy).toHaveBeenCalledTimes(1);
    job.dispose();
    expect(translator.destroy).toHaveBeenCalledTimes(1);
    expect(second.destroy).toHaveBeenCalledTimes(1);
  });

  it("fails clearly for unknown or unsupported detected languages", async () => {
    detector.detect.mockResolvedValue(detection("und", 0.1));
    await expect(start().translate(["An ambiguous fragment"])).rejects.toThrow("could not confidently detect");
    detector.detect.mockResolvedValue(detection("eo"));
    apis.Translator.availability.mockResolvedValue("unavailable");
    await expect(start().translate(["Saluton mondo"])).rejects.toThrow("cannot translate eo");
    expect(apis.Translator.create).not.toHaveBeenCalled();
  });

  it("rejects empty translations and oversized prose before further processing", async () => {
    translator.translate.mockResolvedValue(" ");
    await expect(start().translate(["Some prose"])).rejects.toThrow("empty response");
    detector.detect.mockClear();
    await expect(start().translate(["x".repeat(16 * 1024 + 1)])).rejects.toThrow("batch is too large");
    await expect(start().translate(Array.from({ length: 129 }, () => "text"))).rejects.toThrow("batch is too large");
    await expect(start().translate(Array.from({ length: 5 }, () => "x".repeat(16 * 1024)))).rejects.toThrow("batch is too large");
    expect(detector.detect).not.toHaveBeenCalled();
  });
});

describe("browser model preparation and cancellation", () => {
  it("reports downloads and waits for a new click after activation expires", async () => {
    const progress = vi.fn<(progress: TranslationProgress) => void>();
    let detector_download!: (event: { loaded: number }) => void;
    apis.LanguageDetector.create.mockImplementation(async (options) => {
      options.monitor({ addEventListener: (_, callback) => { detector_download = callback; } });
      return detector;
    });
    const job = start(progress);
    detector_download({ loaded: 0.5 });
    expect(progress).toHaveBeenCalledWith({ message: "Downloading language detection model… 50%" });
    activation.isActive = false;
    apis.Translator.availability.mockResolvedValue("downloadable");
    let translator_download!: (event: { loaded: number }) => void;
    apis.Translator.create.mockImplementation(async (options) => {
      options.monitor({ addEventListener: (_, callback) => { translator_download = callback; } });
      return translator;
    });
    const result = job.translate(["Hello world"]);
    await vi.waitFor(() => expect(progress.mock.calls.some(([value]) => value.resume)).toBe(true));
    expect(apis.Translator.create).not.toHaveBeenCalled();
    const resume = progress.mock.calls.find(([value]) => value.resume)![0].resume!;
    activation.isActive = true;
    resume();
    expect(apis.Translator.create).toHaveBeenCalledTimes(1);
    resume();
    expect(apis.Translator.create).toHaveBeenCalledTimes(1);
    translator_download({ loaded: 0.75 });
    expect(progress).toHaveBeenCalledWith({ message: "Downloading translation model… 75%" });
    expect(await result).toEqual(["译文 Hello world"]);
    job.dispose();
    progress.mockClear();
    translator_download({ loaded: 1 });
    detector_download({ loaded: 1 });
    expect(progress).not.toHaveBeenCalled();
  });

  it("offers a single gesture retry for an available model rejected without activation", async () => {
    const progress = vi.fn<(progress: TranslationProgress) => void>();
    const job = start(progress);
    activation.isActive = false;
    apis.Translator.create.mockRejectedValue(new DOMException("User activation is required", "NotAllowedError"));
    const result = job.translate(["Hello world"]);
    const rejected = expect(result).rejects.toThrow("User activation is required");
    await vi.waitFor(() => expect(progress.mock.calls.some(([value]) => value.resume)).toBe(true));
    progress.mock.calls.find(([value]) => value.resume)![0].resume!();
    expect(apis.Translator.create).toHaveBeenCalledTimes(2);
    await rejected;
    expect(progress.mock.calls.filter(([value]) => value.resume)).toHaveLength(1);
  });

  it("does not retry a policy rejection while activation is present", async () => {
    const progress = vi.fn<(progress: TranslationProgress) => void>();
    apis.Translator.create.mockRejectedValue(new DOMException("Policy denied", "NotAllowedError"));
    await expect(start(progress).translate(["Hello world"])).rejects.toThrow("Policy denied");
    expect(progress.mock.calls.some(([value]) => value.resume)).toBe(false);
  });

  it("unblocks a suspended activation request on disposal and ignores its old callback", async () => {
    const progress = vi.fn<(progress: TranslationProgress) => void>();
    const job = start(progress);
    activation.isActive = false;
    apis.Translator.availability.mockResolvedValue("downloading");
    const result = job.translate(["Hello world"]);
    const rejected = expect(result).rejects.toThrow("cancelled");
    await vi.waitFor(() => expect(progress.mock.calls.some(([value]) => value.resume)).toBe(true));
    const resume = progress.mock.calls.find(([value]) => value.resume)![0].resume!;
    job.dispose();
    await rejected;
    resume();
    expect(apis.Translator.create).not.toHaveBeenCalled();
    expect(detector.destroy).toHaveBeenCalledTimes(1);
  });

  it("aborts pending detector creation immediately and destroys a late-created detector", async () => {
    const pending = deferred<typeof detector>();
    apis.LanguageDetector.create.mockReturnValue(pending.promise);
    const job = start();
    const result = job.translate(["Hello world"]);
    const rejected = expect(result).rejects.toThrow("cancelled");
    job.dispose();
    expect(apis.LanguageDetector.create.mock.calls[0][0].signal.aborted).toBe(true);
    await rejected;
    pending.resolve(detector);
    await vi.waitFor(() => expect(detector.destroy).toHaveBeenCalledTimes(1));
    expect(detector.detect).not.toHaveBeenCalled();
  });

  it("aborts pending translator creation and destroys a late-created translator", async () => {
    const pending = deferred<typeof translator>();
    apis.Translator.create.mockReturnValue(pending.promise);
    const job = start();
    const result = job.translate(["Hello world"]);
    const rejected = expect(result).rejects.toThrow("cancelled");
    await vi.waitFor(() => expect(apis.Translator.create).toHaveBeenCalledTimes(1));
    job.dispose();
    expect(apis.Translator.create.mock.calls[0][0].signal.aborted).toBe(true);
    await rejected;
    pending.resolve(translator);
    await vi.waitFor(() => expect(translator.destroy).toHaveBeenCalledTimes(1));
    expect(translator.translate).not.toHaveBeenCalled();
  });

  it("aborts detection and ignores late results", async () => {
    const pending = deferred<Detection[]>();
    detector.detect.mockReturnValue(pending.promise);
    const job = start();
    const result = job.translate(["Hello world"]);
    const rejected = expect(result).rejects.toThrow("cancelled");
    await vi.waitFor(() => expect(detector.detect).toHaveBeenCalledTimes(1));
    job.dispose();
    expect(detector.detect.mock.calls[0][1].signal.aborted).toBe(true);
    await rejected;
    pending.resolve(detection("en"));
    await Promise.resolve();
    expect(apis.Translator.create).not.toHaveBeenCalled();
  });

  it("aborts translation without publishing late results or retaining model resources", async () => {
    const pending = deferred<string>();
    translator.translate.mockReturnValue(pending.promise);
    const job = start();
    const result = job.translate(["Hello world"]);
    const rejected = expect(result).rejects.toThrow("cancelled");
    await vi.waitFor(() => expect(translator.translate).toHaveBeenCalledTimes(1));
    job.dispose();
    expect(translator.translate.mock.calls[0][1].signal.aborted).toBe(true);
    await rejected;
    pending.resolve("Late translation");
    expect(detector.destroy).toHaveBeenCalledTimes(1);
    expect(translator.destroy).toHaveBeenCalledTimes(1);
    await expect(job.translate(["Another batch"])).rejects.toThrow("cancelled");
  });

  it("observes eager preparation failures even before the caller loads message detail", async () => {
    apis.LanguageDetector.create.mockRejectedValue(new DOMException("Model download failed", "NetworkError"));
    const job = start();
    await new Promise((resolve) => setTimeout(resolve, 0));
    await expect(job.translate(["Hello world"])).rejects.toThrow("Model download failed");
  });

  it("continues cleanup when native destroy throws", async () => {
    const job = start();
    await job.translate(["Hello world"]);
    detector.destroy.mockImplementation(() => { throw new Error("Already released"); });
    expect(() => job.dispose()).not.toThrow();
    expect(translator.destroy).toHaveBeenCalledTimes(1);
  });
});
