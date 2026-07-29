export interface JsonlStats {
  received_lines: number;
  accepted_lines: number;
  malformed_lines: number;
  oversized_lines: number;
}

export class JsonlDecoder<T> {
  readonly stats: JsonlStats = {
    received_lines: 0,
    accepted_lines: 0,
    malformed_lines: 0,
    oversized_lines: 0
  };

  #buffer = "";
  #decoder = new TextDecoder();
  #discardingOversizedLine = false;

  constructor(
    private readonly parse: (value: unknown) => T | null,
    private readonly maxLineLength = 4 * 1024 * 1024
  ) {}

  push(chunk: Uint8Array): T[] {
    this.#buffer += this.#decoder.decode(chunk, { stream: true });
    return this.#drain(false);
  }

  finish(): T[] {
    this.#buffer += this.#decoder.decode();
    return this.#drain(true);
  }

  #drain(final: boolean): T[] {
    const values: T[] = [];

    while (true) {
      const newline = this.#buffer.indexOf("\n");
      if (newline < 0) {
        break;
      }

      const line = this.#buffer.slice(0, newline);
      this.#buffer = this.#buffer.slice(newline + 1);
      if (this.#discardingOversizedLine) {
        this.#discardingOversizedLine = false;
        continue;
      }
      this.#parseLine(line, values);
    }

    if (!this.#discardingOversizedLine && this.#buffer.length > this.maxLineLength) {
      this.stats.received_lines += 1;
      this.stats.oversized_lines += 1;
      this.#buffer = "";
      this.#discardingOversizedLine = true;
    }

    if (this.#discardingOversizedLine && this.#buffer.length > 0) {
      this.#buffer = "";
    }

    if (final && !this.#discardingOversizedLine && this.#buffer.length > 0) {
      this.#parseLine(this.#buffer, values);
      this.#buffer = "";
    }

    return values;
  }

  #parseLine(rawLine: string, values: T[]): void {
    const line = rawLine.endsWith("\r") ? rawLine.slice(0, -1) : rawLine;
    if (line.trim().length === 0) {
      return;
    }

    this.stats.received_lines += 1;
    if (line.length > this.maxLineLength) {
      this.stats.oversized_lines += 1;
      return;
    }
    try {
      const parsed = this.parse(JSON.parse(line));
      if (parsed) {
        this.stats.accepted_lines += 1;
        values.push(parsed);
      } else {
        this.stats.malformed_lines += 1;
      }
    } catch {
      this.stats.malformed_lines += 1;
    }
  }
}

export async function consumeJsonl<T>(
  stream: ReadableStream<Uint8Array>,
  decoder: JsonlDecoder<T>,
  onValue: (value: T) => void,
  signal?: AbortSignal
): Promise<void> {
  const reader = stream.getReader();
  const cancel = (): void => {
    void reader.cancel("JSONL consumption aborted").catch(() => {});
  };
  signal?.addEventListener("abort", cancel, { once: true });
  try {
    if (signal?.aborted) {
      cancel();
    }
    while (true) {
      const result = await reader.read();
      if (result.done) {
        break;
      }
      for (const value of decoder.push(result.value)) {
        onValue(value);
      }
    }
    for (const value of decoder.finish()) {
      onValue(value);
    }
  } finally {
    signal?.removeEventListener("abort", cancel);
    reader.releaseLock();
  }
}
