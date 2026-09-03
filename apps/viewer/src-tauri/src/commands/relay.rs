use tauri::{Manager, State};

use crate::{
  relay::{RelaySettings, RelayStatus},
  service::ViewerService,
};

#[tauri::command]
pub async fn get_relay_status(state: State<'_, ViewerService>) -> Result<RelayStatus, String> {
  Ok(state.relay.status())
}

#[tauri::command]
pub async fn configure_relay(
  app: tauri::AppHandle,
  state: State<'_, ViewerService>,
  settings: RelaySettings,
) -> Result<RelayStatus, String> {
  let _guard = state.relay.configure_lock.lock().await;
  let path = app
    .path()
    .app_config_dir()
    .map_err(|e| e.to_string())?
    .join("relay.json");
  let saved = settings.clone();
  tauri::async_runtime::spawn_blocking(move || crate::relay::write_settings(&path, &saved))
    .await
    .map_err(|e| e.to_string())??;
  state.relay.configure(settings)?;
  // A source-mode change may bring a previously covered provider back to local
  // history; do not leave its catalog waiting for the five-minute safety sweep.
  let _ = state.request_session_index_retry();
  Ok(state.relay.status())
}
