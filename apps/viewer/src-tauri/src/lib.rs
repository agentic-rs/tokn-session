mod commands;
mod model;
mod repository;
mod service;

use service::ViewerService;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .manage(ViewerService::native())
    .invoke_handler(tauri::generate_handler![
      commands::sessions::list_sessions,
      commands::sessions::list_session_children,
      commands::events::load_event_page,
      commands::events::load_event_detail,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tokn session viewer");
}
