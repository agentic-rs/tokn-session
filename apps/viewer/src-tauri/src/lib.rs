mod commands;
mod model;
mod repository;
mod service;

use std::{
  path::{Path, PathBuf},
  time::Duration,
};

use service::ViewerService;
use tauri::{Emitter, Manager};

const INDEX_REFRESH_INTERVAL: Duration = Duration::from_secs(10);

fn session_index_path_for_home(home: &Path) -> PathBuf {
  home.join(".tokn").join("sessions").join("index.sqlite")
}

fn session_index_path() -> Result<PathBuf, String> {
  dirs::home_dir()
    .map(|home| session_index_path_for_home(&home))
    .ok_or_else(|| "could not resolve the user home directory for the session index".to_owned())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .setup(|app| {
      let index_path = session_index_path().expect("viewer session index path should resolve");
      let index_directory = index_path
        .parent()
        .expect("viewer session index path should have a parent directory");
      std::fs::create_dir_all(index_directory).expect("viewer session index directory should be creatable");
      let service = ViewerService::native(index_path).expect("viewer session index should open");
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

#[cfg(test)]
mod tests {
  use std::path::Path;

  use super::session_index_path_for_home;

  #[test]
  fn session_index_uses_the_shared_tokn_sessions_directory() {
    assert_eq!(
      session_index_path_for_home(Path::new("/example/home")),
      Path::new("/example/home/.tokn/sessions/index.sqlite"),
    );
  }
}
