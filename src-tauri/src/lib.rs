use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{SecondsFormat, Utc};
use rand::RngCore;
use reqwest::blocking::Client;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, RunEvent, State, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_opener::OpenerExt;
use toml_edit::{table as toml_table, value as toml_value, DocumentMut};
use url::Url;
use uuid::Uuid;

mod accounts;
mod codex;
mod db;
mod oauth;
mod sessions;
mod state;
mod tray;

use state::*;

#[tauri::command]
fn set_autostart(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    if enabled {
        app.autolaunch()
            .enable()
            .map_err(|error| error.to_string())?
    } else {
        app.autolaunch()
            .disable()
            .map_err(|error| error.to_string())?
    }
    let connection = db::open_database(&state)?;
    db::set_setting(&connection, "autostart_initialized", "true")
}

#[tauri::command]
fn reveal_data_directory(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let parent = state
        .database_path
        .parent()
        .ok_or_else(|| "无法定位应用数据目录。".to_string())?;
    app.opener()
        .open_path(parent.display().to_string(), None::<&str>)
        .map_err(|error| error.to_string())
}

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
            #[cfg(target_os = "windows")]
            app.get_webview_window("main")
                .expect("main window missing")
                .set_decorations(false)?;

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
            app.manage(state);

            let state = app.state::<AppState>();
            let connection = db::open_database(&state)?;
            if db::get_setting(&connection, "autostart_initialized")?.is_none() {
                if let Err(error) = app.autolaunch().enable() {
                    eprintln!("Unable to enable autostart: {error}");
                }
                db::set_setting(&connection, "autostart_initialized", "true")?;
            }

            tray::install_tray(app.handle())?;
            if std::env::args().any(|argument| argument == "--autostart") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            accounts::get_app_status,
            accounts::switch_profile,
            oauth::start_oauth_add,
            oauth::cancel_oauth_add,
            accounts::import_current_profile,
            oauth::import_auth_json,
            accounts::add_relay_profile,
            accounts::refresh_profile_usage,
            accounts::get_profile_auth,
            accounts::update_profile,
            accounts::update_relay_profile,
            accounts::reorder_profiles,
            accounts::delete_profile,
            accounts::set_codex_home,
            codex::get_codex_config,
            codex::validate_codex_config,
            codex::format_codex_config,
            codex::save_codex_config,
            sessions::list_codex_sessions,
            sessions::rename_codex_session,
            sessions::archive_codex_session,
            sessions::restore_codex_session,
            sessions::delete_codex_session,
            set_autostart,
            reveal_data_directory,
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build Tauri application")
        .run(|app, event| match event {
            #[cfg(target_os = "macos")]
            RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } => tray::show_main_window(app),
            RunEvent::ExitRequested { .. } => {
                // The explicit tray quit action is the only normal exit path.
            }
            _ => {}
        });
}
