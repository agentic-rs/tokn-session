use tauri::State;

use crate::model::SessionIndexProgress;
use crate::service::ViewerService;

/// Reads the latest worker snapshot without waiting for SQLite or a provider.
#[tauri::command]
pub async fn get_session_index_progress(state: State<'_, ViewerService>) -> Result<SessionIndexProgress, String> {
  Ok(state.session_index_progress())
}

/// Requests that the setup-owned index scheduler run again. This command only
/// queues a wake; it deliberately does not start a second provider worker on
/// the Tauri IPC task.
#[tauri::command]
pub async fn retry_session_index(state: State<'_, ViewerService>) -> Result<SessionIndexProgress, String> {
  state.request_session_index_retry()
}
