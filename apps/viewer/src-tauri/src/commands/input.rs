use tauri::State;
use tokn_viewer_core::{
  ViewerService,
  model::{SessionInputStatus, SessionInputStatusRequest, SubmitSessionInputRequest, SubmitSessionInputResponse},
};

#[tauri::command]
pub async fn get_session_input_status(
  service: State<'_, ViewerService>,
  request: SessionInputStatusRequest,
) -> Result<SessionInputStatus, String> {
  service.get_session_input_status(request).await
}

#[tauri::command]
pub async fn submit_session_input(
  service: State<'_, ViewerService>,
  request: SubmitSessionInputRequest,
) -> Result<SubmitSessionInputResponse, String> {
  service.submit_session_input(request).await
}
