use super::state::AppState;
use std::{
    path::{Path, PathBuf},
    process::Command,
};
use tauri::State;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_opener::OpenerExt;

pub(crate) fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|error| error.to_string())
}

pub(crate) fn reveal_data_directory(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let parent = state
        .database_path
        .parent()
        .ok_or_else(|| "无法定位应用数据目录。".to_string())?;
    app.opener()
        .open_path(parent.display().to_string(), None::<&str>)
        .map_err(|error| error.to_string())
}

pub(crate) fn open_codex_home(app: tauri::AppHandle, codex_home: String) -> Result<bool, String> {
    let path = existing_directory(&codex_home)?;
    if open_with_vscode(&path) {
        return Ok(true);
    }
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|error| error.to_string())?;
    Ok(false)
}

pub(crate) fn open_codex_cli_install_page(app: tauri::AppHandle) -> Result<(), String> {
    open_url(app, "https://learn.chatgpt.com/docs/codex/cli")
}

pub(crate) fn open_claude_cli_install_page(app: tauri::AppHandle) -> Result<(), String> {
    open_url(app, "https://code.claude.com/docs/en/quickstart")
}

pub(crate) fn open_antigravity_cli_install_page(app: tauri::AppHandle) -> Result<(), String> {
    open_url(
        app,
        "https://github.com/google-antigravity/antigravity-cli/releases/latest",
    )
}

pub(crate) fn open_grok_cli_install_page(app: tauri::AppHandle) -> Result<(), String> {
    open_url(app, "https://x.ai/cli/stable")
}

fn open_url(app: tauri::AppHandle, url: &str) -> Result<(), String> {
    app.opener()
        .open_url(url, None::<&str>)
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
