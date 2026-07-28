import { describe, expect, test } from "bun:test";

import { PetStore } from "../src/state";
import { relayEvent } from "./fixtures";

const policy = {
  ready_debounce_ms: 10,
  ready_hold_ms: 50,
  error_grace_ms: 20,
  running_lease_ms: 100,
  open_tool_lease_ms: 200,
  blocked_lease_ms: 300,
  input_lease_ms: 400
};

describe("PetStore", () => {
  test("moves from user work to debounced ready and then idle", () => {
    const store = new PetStore(policy);
    store.ingest(relayEvent({
      type: "message",
      role: "user",
      phase: "finished",
      text: "build it"
    }), 0);
    expect(store.snapshot(1).state).toBe("running");

    store.ingest(relayEvent({
      type: "message",
      role: "assistant",
      phase: "finished",
      text: "done"
    }), 20);
    expect(store.snapshot(29).state).toBe("running");
    expect(store.snapshot(30).state).toBe("ready");
    expect(store.snapshot(80).state).toBe("idle");
  });

  test("does not report ready while a tool remains open", () => {
    const store = new PetStore(policy);
    store.ingest(relayEvent({
      type: "tool_call",
      tool_call_id: "call-1",
      tool_name: "exec_command",
      phase: "started",
      input: { cmd: "cargo test" }
    }), 0);
    store.ingest(relayEvent({
      type: "message",
      role: "assistant",
      phase: "finished",
      text: "premature"
    }), 10);
    expect(store.snapshot(30).state).toBe("running");

    store.ingest(relayEvent({
      type: "tool_call",
      tool_call_id: "call-1",
      tool_name: "exec_command",
      phase: "finished",
      output: { exit_code: 0 }
    }), 40);
    store.ingest(relayEvent({
      type: "message",
      role: "assistant",
      phase: "finished",
      text: "done"
    }), 50);
    expect(store.snapshot(60).state).toBe("ready");
  });

  test("turns a stable error into blocked but cancels it on progress", () => {
    const store = new PetStore(policy);
    store.ingest(relayEvent({
      type: "error",
      message: "stream failed"
    }), 0);
    expect(store.snapshot(19).state).toBe("idle");
    expect(store.snapshot(20).state).toBe("blocked");

    store.ingest(relayEvent({
      type: "reasoning",
      phase: "delta",
      text: "recovering"
    }), 30);
    expect(store.snapshot(31).state).toBe("running");
  });

  test("prioritizes needs input and clears it when work resumes", () => {
    const store = new PetStore(policy);
    store.ingest(relayEvent({
      type: "unknown",
      native_type: "event_msg.request_user_input",
      native: {
        id: "question-1"
      }
    }), 0);
    expect(store.snapshot(1).state).toBe("needs_input");

    store.ingest(relayEvent({
      type: "message",
      role: "user",
      phase: "finished",
      text: "yes"
    }), 10);
    expect(store.snapshot(11).state).toBe("running");
  });

  test("uses official state priority across sessions", () => {
    const store = new PetStore(policy);
    store.ingest(relayEvent({
      type: "reasoning",
      phase: "delta"
    }, "codex.running"), 0);
    store.ingest(relayEvent({
      type: "message",
      role: "assistant",
      phase: "finished",
      text: "ready"
    }, "codex.ready"), 0);
    store.ingest(relayEvent({
      type: "unknown",
      native_type: "event_msg.exec_approval_request",
      native: {
        approval_id: "approval-1"
      }
    }, "codex.waiting"), 0);

    const snapshot = store.snapshot(10);
    expect(snapshot.state).toBe("needs_input");
    expect(snapshot.focus?.topic).toBe("codex.waiting");
    expect(snapshot.active_sessions).toBe(3);
  });

  test("keeps recoverable tool errors in running", () => {
    const store = new PetStore(policy);
    store.ingest(relayEvent({
      type: "tool_call",
      tool_call_id: "call-1",
      tool_name: "exec_command",
      phase: "finished",
      output: { exit_code: 1 },
      is_error: true
    }), 0);
    expect(store.snapshot(1).state).toBe("running");
  });
});
