import { useLayoutEffect, useRef } from "react";

interface Anchor {
  key: string;
  top: number;
}

interface ScrollPosition {
  top: number;
  height: number;
  viewport: number;
  anchors: Anchor[];
}

interface TimelineScrollOptions {
  session_key: string | null;
  initial_page_loaded: boolean;
  on_follow_change?: (following: boolean) => void;
}

const BOTTOM_THRESHOLD = 48;

/** One owner for both React updates and asynchronous changes to layout. */
export function useTimelineScroll({ session_key, initial_page_loaded, on_follow_change }: TimelineScrollOptions) {
  const timelineRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const inputFrame = useRef<number | null>(null);
  const observation = useRef<{
    observer: ResizeObserver;
    timeline: HTMLDivElement;
    content: HTMLDivElement | null;
  } | null>(null);
  const state = useRef({
    session_key,
    initialized: false,
    following: true,
    user_scroll: false,
    position: null as ScrollPosition | null,
  });
  const followCallback = useRef(on_follow_change);
  followCallback.current = on_follow_change;

  function setFollowing(following: boolean) {
    if (state.current.following === following) return;
    state.current.following = following;
    followCallback.current?.(following);
  }

  function capture() {
    const timeline = timelineRef.current;
    if (!timeline) return;
    const viewport = timeline.getBoundingClientRect();
    const visible = [...timeline.querySelectorAll<HTMLElement>("[data-scroll-key]")]
      .map((element) => ({ element, rect: element.getBoundingClientRect() }))
      .filter(({ rect }) => rect.bottom > viewport.top && rect.top < viewport.bottom);
    // Prefer the smallest row crossing the reading line. A turn can span many
    // screens; anchoring its outer box misses changes above a visible child.
    const ancestors = new Set<HTMLElement>();
    for (const { element } of visible) {
      for (let parent = element.parentElement; parent && parent !== timeline; parent = parent.parentElement) {
        ancestors.add(parent);
      }
    }
    const leaves = visible.filter(({ element }) => !ancestors.has(element));
    const crossing = leaves.filter(({ rect }) => rect.top <= viewport.top)
      .sort((left, right) => left.rect.height - right.rect.height);
    const below = leaves.filter(({ rect }) => rect.top > viewport.top);
    const fallback = visible.filter(({ element }) => ancestors.has(element));
    state.current.position = {
      top: timeline.scrollTop,
      height: timeline.scrollHeight,
      viewport: timeline.clientHeight,
      anchors: [...crossing, ...below, ...fallback].map(({ element, rect }) => ({
        key: element.dataset.scrollKey!, top: rect.top - viewport.top,
      })),
    };
  }

  function acceptUserScroll(): boolean {
    const timeline = timelineRef.current;
    const previous = state.current.position;
    if (!timeline || !previous) return false;
    const movement = timeline.scrollTop - previous.top;
    const resized = timeline.scrollHeight !== previous.height || timeline.clientHeight !== previous.viewport;
    const bottom = Math.max(0, timeline.scrollHeight - timeline.clientHeight);
    const clamped = resized && previous.top > bottom && Math.abs(timeline.scrollTop - bottom) < 1;
    if (Math.abs(movement) <= 0.5 || clamped || (resized && !state.current.user_scroll)) return false;
    state.current.user_scroll = false;
    // Even a small upward gesture pauses following. The proximity threshold
    // only resumes it when the reader actually moves down toward the end.
    setFollowing(movement > 0 && bottom - timeline.scrollTop < BOTTOM_THRESHOLD);
    capture();
    return true;
  }

  function reconcile() {
    const timeline = timelineRef.current;
    if (!timeline || !state.current.initialized) return;
    // A commit can happen after native scrolling but before its scroll event.
    if (state.current.user_scroll && acceptUserScroll()) return;
    const previous = state.current.position;
    if (state.current.following) {
      timeline.scrollTop = Math.max(0, timeline.scrollHeight - timeline.clientHeight);
    } else if (previous) {
      const elements = new Map([...timeline.querySelectorAll<HTMLElement>("[data-scroll-key]")]
        .map((element) => [element.dataset.scrollKey!, element]));
      const anchor = previous.anchors.find(({ key }) => elements.has(key));
      if (anchor) {
        const offset = elements.get(anchor.key)!.getBoundingClientRect().top - timeline.getBoundingClientRect().top;
        const desired = timeline.scrollTop + offset - anchor.top;
        timeline.scrollTop = Math.max(0, Math.min(desired, timeline.scrollHeight - timeline.clientHeight));
      } else {
        timeline.scrollTop = Math.max(0, Math.min(previous.top, timeline.scrollHeight - timeline.clientHeight));
      }
    }
    capture();
  }

  // Run after every parent commit, including loading banners and expanded detail.
  // Child-only changes (translation, font reflow) are covered by ResizeObserver.
  useLayoutEffect(() => {
    if (state.current.session_key !== session_key) {
      state.current = { session_key, initialized: false, following: true, user_scroll: false, position: null };
    }
    if (session_key && initial_page_loaded) state.current.initialized = true;
    reconcile();
  });

  useLayoutEffect(() => () => {
    if (inputFrame.current !== null) cancelAnimationFrame(inputFrame.current);
    observation.current?.observer.disconnect();
    observation.current = null;
  }, []);

  const reconcileRef = useRef(reconcile);
  reconcileRef.current = reconcile;
  useLayoutEffect(() => {
    const timeline = timelineRef.current;
    const content = contentRef.current;
    if (observation.current?.timeline === timeline && observation.current?.content === content) return;
    observation.current?.observer.disconnect();
    observation.current = null;
    if (!timeline || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => reconcileRef.current());
    observer.observe(timeline);
    if (content) observer.observe(content);
    observation.current = { observer, timeline, content };
  });

  function onScroll() {
    const timeline = timelineRef.current;
    if (!timeline || !state.current.initialized) return;
    if (!acceptUserScroll()) {
      // Programmatic scrolling and browser clamping after a shrinking card must
      // not turn reading mode back into follow mode (or vice versa).
      reconcile();
    }
  }

  function noteUserScroll(upward = false) {
    state.current.user_scroll = true;
    if (upward) setFollowing(false);
    if (inputFrame.current !== null) cancelAnimationFrame(inputFrame.current);
    inputFrame.current = requestAnimationFrame(() => {
      inputFrame.current = null;
      state.current.user_scroll = false;
    });
  }

  function pause() {
    state.current.user_scroll = false;
    setFollowing(false);
    capture();
  }

  function jumpToLatest() {
    state.current.user_scroll = false;
    setFollowing(true);
    reconcile();
  }

  return { timelineRef, contentRef, onScroll, noteUserScroll, pause, jumpToLatest };
}
