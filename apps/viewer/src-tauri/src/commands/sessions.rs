use tauri::State;

use crate::model::{
  ListSessionChildrenRequest, ListSessionChildrenResponse, ListSessionsRequest, ListSessionsResponse,
};
use crate::service::ViewerService;

#[tauri::command]
pub async fn list_sessions(
  state: State<'_, ViewerService>,
  request: ListSessionsRequest,
) -> Result<ListSessionsResponse, String> {
  let service = state.inner().clone();
  tauri::async_runtime::spawn_blocking(move || service.list_sessions(request))
    .await
    .map_err(|error| format!("session listing task failed: {error}"))?
}

#[tauri::command]
pub async fn list_session_children(
  state: State<'_, ViewerService>,
  request: ListSessionChildrenRequest,
) -> Result<ListSessionChildrenResponse, String> {
  let service = state.inner().clone();
  tauri::async_runtime::spawn_blocking(move || service.list_session_children(request))
    .await
    .map_err(|error| format!("session-child listing task failed: {error}"))?
}
