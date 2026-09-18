import { isRecord } from "../lib/ipc-protocol";

/**
 * Plain-text subset of Desktop 26.901.41123's user-input projection.
 * The owner retains request.input verbatim before app-server normalization;
 * its renderer reads text_elements.length for every text part (Snn via xrn
 * in app-initial-236e1501144c.js). A router acknowledgement alone misses this.
 */
export function renderDesktopPlainText(input: unknown): string {
  if (!Array.isArray(input)) throw new Error("Desktop input must be an array");
  return input.map((part: unknown) => {
    if (!isRecord(part) || part.type !== "text" || typeof part.text !== "string") {
      throw new Error("The lab supports only Desktop plain-text input");
    }
    if (!Array.isArray(part.text_elements)) {
      throw new Error("Desktop text input requires a text_elements array");
    }
    if (part.text_elements.length !== 0) {
      throw new Error("The lab does not project annotated Desktop text input");
    }
    return part.text;
  }).join("\n");
}
