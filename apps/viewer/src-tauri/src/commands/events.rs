use tauri::State;

use crate::model::{
  EventDetail, EventPage, EventPageRequest, LoadEventDetailRequest, LoadTrajectoryEventPageRequest, TrajectoryEventPage,
};
use crate::service::ViewerService;

#[tauri::command]
pub async fn load_event_page(state: State<'_, ViewerService>, request: EventPageRequest) -> Result<EventPage, String> {
  let service = state.inner().clone();
  tauri::async_runtime::spawn_blocking(move || service.load_event_page(request))
    .await
    .map_err(|error| format!("event loading task failed: {error}"))?
}

#[tauri::command]
pub async fn load_event_detail(
  state: State<'_, ViewerService>,
  request: LoadEventDetailRequest,
) -> Result<EventDetail, String> {
  let service = state.inner().clone();
  tauri::async_runtime::spawn_blocking(move || service.load_event_detail(request))
    .await
    .map_err(|error| format!("event detail task failed: {error}"))?
}

#[tauri::command]
pub async fn load_trajectory_event_page(
  state: State<'_, ViewerService>,
  request: LoadTrajectoryEventPageRequest,
) -> Result<TrajectoryEventPage, String> {
  let service = state.inner().clone();
  tauri::async_runtime::spawn_blocking(move || service.load_trajectory_event_page(request))
    .await
    .map_err(|error| format!("trajectory event loading task failed: {error}"))?
}
