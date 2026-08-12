import {
  CodexInputBroker,
  codexInputAdmissionStatus,
  type CodexInputAdmission,
  type CodexInputBrokerOptions,
} from "./codex_input";
import {
  PiInputBroker,
  piInputAdmissionStatus,
  type PiInputAdmission,
  type PiInputBrokerOptions,
} from "./input";
import type { RelayEvent } from "./protocol";

export type SessionInputAdmission = PiInputAdmission | CodexInputAdmission;

export interface SessionInputBrokerOptions {
  pi?: PiInputBrokerOptions;
  codex?: CodexInputBrokerOptions;
}

export class SessionInputBroker {
  readonly #pi: PiInputBroker;
  readonly #codex: CodexInputBroker;

  constructor(options: SessionInputBrokerOptions = {}) {
    this.#pi = new PiInputBroker(options.pi);
    this.#codex = new CodexInputBroker(options.codex);
  }

  observe(event: RelayEvent): void {
    this.#pi.observe(event);
    this.#codex.observe(event);
  }

  submit(topic: string, prompt: string): Promise<SessionInputAdmission> {
    const provider = providerFromTopic(topic);
    if (provider === "pi") {
      return this.#pi.submit(topic, prompt);
    }
    if (provider === "codex") {
      return this.#codex.submit(topic, prompt);
    }
    return Promise.reject(new Error(`input is not supported for ${provider || "this provider"}`));
  }
}

export function sessionInputAdmissionStatus(admission: SessionInputAdmission): string {
  return isCodexAdmission(admission)
    ? codexInputAdmissionStatus(admission)
    : piInputAdmissionStatus(admission);
}

function isCodexAdmission(
  admission: SessionInputAdmission
): admission is CodexInputAdmission {
  return "route" in admission && admission.provider === "codex";
}

export function sessionInputProviderLabel(topic: string): string {
  const provider = providerFromTopic(topic);
  switch (provider) {
    case "pi":
      return "Pi";
    case "codex":
      return "Codex";
    default:
      return provider || "session";
  }
}

function providerFromTopic(topic: string): string {
  const separator = topic.indexOf(".");
  return (separator < 0 ? topic : topic.slice(0, separator)).trim().toLowerCase();
}
