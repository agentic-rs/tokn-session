mod commands;
mod model;
mod repository;
mod service;

use std::time::Duration;

use service::ViewerService;
use tauri::{Emitter, Manager};

const INDEX_REFRESH_INTERVAL: Duration = Duration::from_secs(10);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .setup(|app| {
      let app_data_dir = app
        .path()
        .app_local_data_dir()
        .expect("viewer app-local data directory should resolve");
      std::fs::create_dir_all(&app_data_dir).expect("viewer app-local data directory should be creatable");
      let service =
        ViewerService::native(app_data_dir.join("session-index.sqlite3")).expect("viewer session index should open");
      let refresh_service = service.clone();
      let app_handle = app.handle().clone();
      app.manage(service);

      // Initial discovery is deliberately background-only: an established
      // sidebar remains responsive from its previous index, while a first run
      // retains native header discovery until its baseline commits.
      tauri::async_runtime::spawn(async move {
        loop {
          let service = refresh_service.clone();
          match tauri::async_runtime::spawn_blocking(move || service.refresh_session_index()).await {
            Ok(Ok(refresh)) if refresh.changed => {
              let _ = app_handle.emit("session-index-changed", refresh);
            }
            Ok(Ok(_)) => {}
            Ok(Err(error)) => eprintln!("viewer session index refresh failed: {error}"),
            Err(error) => eprintln!("viewer session index refresh task failed: {error}"),
          }
          tokio::time::sleep(INDEX_REFRESH_INTERVAL).await;
        }
      });
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      commands::sessions::list_sessions,
      commands::sessions::list_session_children,
      commands::events::load_event_page,
      commands::events::load_event_detail,
      commands::events::load_trajectory_event_page,
      commands::events::acknowledge_session_attention,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tokn session viewer");
}
