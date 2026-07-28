const ESC = "\u001b";

export interface TerminalWriter {
  write(value: string): boolean;
}

export class TerminalSurface {
  #active = false;
  #previousLines: string[] = [];

  constructor(private readonly writer: TerminalWriter = process.stdout) {}

  enter(): void {
    if (this.#active) {
      return;
    }
    this.#active = true;
    this.#previousLines = [];
    this.writer.write(`${ESC}[?1049h${ESC}[?25l${ESC}[2J${ESC}[H`);
  }

  render(lines: string[]): void {
    if (!this.#active) {
      return;
    }
    let output = "";
    const rowCount = Math.max(lines.length, this.#previousLines.length);
    for (let index = 0; index < rowCount; index += 1) {
      const line = lines[index] ?? "";
      if (line === this.#previousLines[index]) {
        continue;
      }
      output += `${ESC}[${index + 1};1H${ESC}[2K${line}`;
    }
    this.#previousLines = [...lines];
    if (output) {
      this.writer.write(output);
    }
  }

  invalidate(): void {
    if (!this.#active) {
      return;
    }
    this.#previousLines = [];
    this.writer.write(`${ESC}[2J${ESC}[H`);
  }

  leave(): void {
    if (!this.#active) {
      return;
    }
    this.#active = false;
    this.#previousLines = [];
    this.writer.write(`${ESC}[0m${ESC}[?25h${ESC}[?1049l`);
  }
}
