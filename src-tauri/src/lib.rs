use std::{
    fs,
    sync::{Arc, Mutex},
};
use tauri::{Manager, RunEvent, WindowEvent};

mod features;
mod platform;
mod products;

use features::accounts;
use platform::{db, local_web, state::AppState, tray};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("Cortana")
                .arg("--autostart")
                .build(),
        )
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            tray::show_main_window(app);
        }))
        .setup(|app| {
            let home_dir = app.path().home_dir().map_err(|error| error.to_string())?;
            let data_dir = home_dir.join(".cortana");
            fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
            let database_path = data_dir.join("app.sqlite3");
            let default_codex_home = home_dir.join(".codex");
            let state = AppState {
                database_path,
                default_codex_home,
                pending_oauth: Arc::new(Mutex::new(None)),
            };
            db::initialize_database(&state)?;
            accounts::start_usage_refresh_scheduler(state.clone());
            app.manage(state);

            local_web::initialize(app.handle())?;

            tray::install_tray(app.handle())?;
            if std::env::args().any(|argument| argument == "--autostart") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![local_web::invoke_local])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build Tauri application")
        .run(|_app, event| match event {
            #[cfg(target_os = "macos")]
            RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } => tray::show_main_window(_app),
            RunEvent::ExitRequested { .. } => {
                // The explicit tray quit action is the only normal exit path.
            }
            _ => {}
        });
}
