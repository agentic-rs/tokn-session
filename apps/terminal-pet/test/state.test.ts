import { describe, expect, test } from "bun:test";

import { PetStore } from "../src/state";
import { relayEvent } from "./fixtures";

const policy = {
  ready_debounce_ms: 10,
  ready_hold_ms: 50,
  recent_completion_ms: 500,
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
    const completed = store.snapshot(80);
    expect(completed.state).toBe("idle");
    expect(completed.sessions[0]).toMatchObject({
      completed_at: 30,
      recently_completed: true
    });

    store.ingest(relayEvent({
      type: "message",
      role: "user",
      phase: "finished",
      text: "one more thing"
    }), 90);
    expect(store.snapshot(91).sessions[0]).toMatchObject({
      state: "running",
      recently_completed: false
    });
    expect(store.snapshot(91).sessions[0]?.completed_at).toBeUndefined();
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
      type: "reasoning",
      phase: "delta",
      text: "background progress"
    }), 5);
    expect(store.snapshot(6).state).toBe("needs_input");

    store.acknowledge("codex.session-1");
    expect(store.snapshot(7).state).toBe("needs_input");

    store.ingest(relayEvent({
      type: "message",
      role: "assistant",
      phase: "finished",
      text: "still waiting"
    }), 8);
    expect(store.snapshot(9).state).toBe("needs_input");

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
    expect(snapshot.sessions.map((session) => session.topic)).toEqual([
      "codex.waiting",
      "codex.ready",
      "codex.running"
    ]);
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

  test("lists active sessions before retained recent completions", () => {
    const store = new PetStore(policy);
    store.ingest(relayEvent({
      type: "message",
      role: "assistant",
      phase: "finished",
      text: "done"
    }, "codex.completed"), 0);
    store.ingest(relayEvent({
      type: "reasoning",
      phase: "delta",
      text: "working"
    }, "codex.running"), 20);
    store.ingest(relayEvent({
      type: "session_started"
    }, "codex.inactive"), 30);

    const withActive = store.snapshot(70);
    expect(withActive.sessions.map((session) => session.topic)).toEqual([
      "codex.running",
      "codex.completed"
    ]);
    expect(withActive.sessions[1]).toMatchObject({
      state: "idle",
      completed_at: 10,
      recently_completed: true
    });
    expect(withActive.focus?.topic).toBe("codex.running");
    expect(withActive.active_sessions).toBe(1);
    expect(withActive.total_sessions).toBe(3);

    const onlyRecent = store.snapshot(150);
    expect(onlyRecent.sessions.map((session) => session.topic)).toEqual([
      "codex.completed"
    ]);
    expect(onlyRecent.focus?.topic).toBe("codex.completed");
    expect(onlyRecent.state).toBe("idle");

    const expired = store.snapshot(510);
    expect(expired.sessions).toEqual([]);
    expect(expired.focus).toBeUndefined();
    expect(expired.total_sessions).toBe(3);
  });

  test("sorts active and recent sessions deterministically", () => {
    const store = new PetStore(policy);
    store.ingest(relayEvent({
      type: "reasoning",
      phase: "delta"
    }, "codex.running-z"), 0);
    store.ingest(relayEvent({
      type: "reasoning",
      phase: "delta"
    }, "codex.running-a"), 0);
    store.ingest(relayEvent({
      type: "goal_updated",
      goal: { status: "blocked" }
    }, "codex.blocked"), 0);
    store.ingest(relayEvent({
      type: "unknown",
      native_type: "event_msg.request_user_input",
      native: { id: "question-1" }
    }, "codex.waiting"), 0);

    expect(store.snapshot(1).sessions.map((session) => session.topic)).toEqual([
      "codex.waiting",
      "codex.blocked",
      "codex.running-a",
      "codex.running-z"
    ]);

    store.ingest(relayEvent({
      type: "message",
      role: "assistant",
      phase: "finished",
      text: "older"
    }, "codex.completed-old"), 10);
    store.ingest(relayEvent({
      type: "message",
      role: "assistant",
      phase: "finished",
      text: "newer"
    }, "codex.completed-new"), 20);

    expect(store.snapshot(450).sessions.map((session) => session.topic)).toEqual([
      "codex.completed-new",
      "codex.completed-old"
    ]);
  });

  test("acknowledging a completion removes it from the recent roster", () => {
    const store = new PetStore(policy);
    store.ingest(relayEvent({
      type: "message",
      role: "assistant",
      phase: "finished",
      text: "done"
    }), 0);
    expect(store.snapshot(20).sessions).toHaveLength(1);

    store.acknowledge("codex.session-1");

    const snapshot = store.snapshot(20);
    expect(snapshot.sessions).toEqual([]);
    expect(snapshot.focus).toBeUndefined();
    expect(snapshot.total_sessions).toBe(1);
  });
});
