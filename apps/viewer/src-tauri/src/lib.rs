mod commands;
mod model;
mod repository;
mod service;

use std::{
  path::{Path, PathBuf},
  time::{Duration, Instant},
};

use service::ViewerService;
use tauri::{Emitter, Manager};

const INDEX_REFRESH_INTERVAL: Duration = Duration::from_secs(10);
const INDEX_PENDING_BODY_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const INDEX_CATALOG_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const MAX_CONSECUTIVE_CATALOG_RETRIES: u8 = 2;

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
      // exposes its compact catalog as soon as the catalog pass commits.
      tauri::async_runtime::spawn(async move {
        let mut next_catalog_refresh = Instant::now();
        let mut consecutive_catalog_retries = 0_u8;
        loop {
          let catalog_due = Instant::now() >= next_catalog_refresh;
          let service = refresh_service.clone();
          let result = tauri::async_runtime::spawn_blocking(move || {
            if catalog_due {
              service.refresh_session_catalog()
            } else {
              service.refresh_pending_session_index()
            }
          })
          .await;
          match result {
            Ok(Ok(refresh)) => {
              // Start the ordinary catalog interval after its potentially slow
              // blocking scan returns. A structurally changing inventory is a
              // normal race, not a provider error, so make up to two quick
              // retries before returning to the regular cadence. This avoids
              // a long-lived active provider becoming a one-second full-scan
              // loop.
              if catalog_due {
                let retry_soon =
                  refresh.retry_catalog_soon && consecutive_catalog_retries < MAX_CONSECUTIVE_CATALOG_RETRIES;
                if retry_soon {
                  consecutive_catalog_retries += 1;
                  next_catalog_refresh = Instant::now() + INDEX_CATALOG_RETRY_INTERVAL;
                } else {
                  consecutive_catalog_retries = 0;
                  next_catalog_refresh = Instant::now() + INDEX_REFRESH_INTERVAL;
                }
              }
              let has_pending_body_jobs = refresh.has_pending_body_jobs;
              if refresh.changed {
                let _ = app_handle.emit("session-index-changed", refresh);
              }
              let until_catalog = next_catalog_refresh.saturating_duration_since(Instant::now());
              let delay = if has_pending_body_jobs {
                INDEX_PENDING_BODY_REFRESH_INTERVAL.min(until_catalog)
              } else {
                until_catalog
              };
              tokio::time::sleep(delay).await;
            }
            Ok(Err(error)) => {
              eprintln!("viewer session index refresh failed: {error}");
              if catalog_due {
                consecutive_catalog_retries = 0;
                next_catalog_refresh = Instant::now() + INDEX_REFRESH_INTERVAL;
              }
              tokio::time::sleep(next_catalog_refresh.saturating_duration_since(Instant::now())).await;
            }
            Err(error) => {
              eprintln!("viewer session index refresh task failed: {error}");
              if catalog_due {
                consecutive_catalog_retries = 0;
                next_catalog_refresh = Instant::now() + INDEX_REFRESH_INTERVAL;
              }
              tokio::time::sleep(next_catalog_refresh.saturating_duration_since(Instant::now())).await;
            }
          }
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
