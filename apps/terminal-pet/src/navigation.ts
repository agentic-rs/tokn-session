import {
  effectivePetState,
  effectiveStateChangedAt,
  type PetSnapshot
} from "./state";

export type FocusDirection = "next" | "previous";

export function focusSnapshot(
  snapshot: PetSnapshot,
  topic: string | undefined
): PetSnapshot {
  if (!topic) {
    return snapshot;
  }
  const focus = snapshot.sessions.find((session) => session.topic === topic);
  if (!focus) {
    return snapshot;
  }
  return {
    ...snapshot,
    state: effectivePetState(focus),
    state_changed_at: effectiveStateChangedAt(focus),
    focus
  };
}

export function moveFocusTopic(
  snapshot: PetSnapshot,
  currentTopic: string | undefined,
  direction: FocusDirection
): string | undefined {
  if (snapshot.sessions.length === 0) {
    return undefined;
  }
  if (!currentTopic && snapshot.sessions.length === 1) {
    return undefined;
  }
  const automaticTopic = snapshot.focus?.topic;
  const activeTopic = currentTopic ?? automaticTopic;
  const currentIndex = snapshot.sessions.findIndex(
    (session) => session.topic === activeTopic
  );
  if (currentIndex < 0) {
    return direction === "next"
      ? snapshot.sessions[0]?.topic
      : snapshot.sessions.at(-1)?.topic;
  }
  const offset = direction === "next" ? 1 : -1;
  const nextIndex = (
    currentIndex
    + offset
    + snapshot.sessions.length
  ) % snapshot.sessions.length;
  return snapshot.sessions[nextIndex]?.topic;
}
