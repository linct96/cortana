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
    process::Command,
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
mod agents;
mod analytics;
mod billing;
mod codex;
mod db;
mod env;
mod local_web;
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

#[tauri::command]
fn open_codex_home(app: tauri::AppHandle, codex_home: String) -> Result<bool, String> {
    let path = existing_directory(&codex_home)?;
    if open_with_vscode(&path) {
        return Ok(true);
    }
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|error| error.to_string())?;
    Ok(false)
}

#[tauri::command]
fn open_codex_cli_install_page(app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_url("https://learn.chatgpt.com/docs/codex/cli", None::<&str>)
        .map_err(|error| error.to_string())
}

fn existing_directory(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value.trim());
    if path.is_dir() {
        Ok(path)
    } else {
        Err("Codex 主目录不存在。".to_string())
    }
}

#[cfg(target_os = "macos")]
fn open_with_vscode(path: &Path) -> bool {
    Command::new("/usr/bin/open")
        .args(["-a", "Visual Studio Code"])
        .arg(path)
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "windows")]
fn open_with_vscode(path: &Path) -> bool {
    let mut candidates = vec![PathBuf::from("code.exe")];
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local_app_data).join("Programs/Microsoft VS Code/Code.exe"));
    }
    candidates.into_iter().any(|candidate| {
        Command::new(candidate)
            .arg(path)
            .status()
            .is_ok_and(|status| status.success())
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_with_vscode(path: &Path) -> bool {
    Command::new("code")
        .arg(path)
        .status()
        .is_ok_and(|status| status.success())
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

            local_web::initialize(app.handle())?;

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
        .invoke_handler(tauri::generate_handler![local_web::invoke_local])
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
