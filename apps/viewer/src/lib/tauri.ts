import { invoke } from "@tauri-apps/api/core";
import type {
  EventPageResponse,
  ListSessionsRequest,
  ListSessionsResponse,
  LoadEventDetailRequest,
  EventDetail,
  LoadEventPageRequest,
} from "./types";

export function listSessions(request: ListSessionsRequest): Promise<ListSessionsResponse> {
  return invoke<ListSessionsResponse>("list_sessions", { request });
}

export function loadEventPage(request: LoadEventPageRequest): Promise<EventPageResponse> {
  return invoke<EventPageResponse>("load_event_page", { request });
}

export function loadEventDetail(request: LoadEventDetailRequest): Promise<EventDetail> {
  return invoke<EventDetail>("load_event_detail", { request });
}
