use crate::platform::{
    db,
    state::{AppState, OAuthProgress, OAuthProgressSnapshot, WebAccessStatus},
    tray,
};
use serde_json::Value;
use std::sync::Mutex;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

mod dispatch;
mod server;

use dispatch::dispatch_command;
use server::{effective_web_port, start_server, RunningServer};

pub(crate) const DEFAULT_WEB_PORT: u16 = 11456;
const MIN_WEB_PORT: u16 = 1024;

pub(crate) struct WebBridgeState {
    runtime: Mutex<Option<RunningServer>>,
    last_error: Mutex<Option<String>>,
    update_lock: Mutex<()>,
    oauth_progress: Mutex<OAuthProgressSnapshot>,
}

pub(crate) fn initialize(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut connection = db::open_database(&state)?;
    let enabled = match db::get_setting(&connection, "web_access_enabled")?.as_deref() {
        Some("true") => true,
        Some("false") | None => false,
        Some(_) => {
            db::set_web_access_settings(&mut connection, false, DEFAULT_WEB_PORT)?;
            false
        }
    };
    let stored_port = db::get_setting(&connection, "web_access_port")?;
    let port = effective_web_port(
        stored_port
            .as_deref()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port >= MIN_WEB_PORT)
            .unwrap_or(DEFAULT_WEB_PORT),
    );
    if !cfg!(debug_assertions) && stored_port.as_deref() != Some(port.to_string().as_str()) {
        db::set_web_access_settings(&mut connection, enabled, port)?;
    }
    drop(connection);

    app.manage(WebBridgeState {
        runtime: Mutex::new(None),
        last_error: Mutex::new(None),
        update_lock: Mutex::new(()),
        oauth_progress: Mutex::new(OAuthProgressSnapshot {
            sequence: 0,
            pending: false,
            progress: None,
        }),
    });

    if enabled {
        let bridge = app.state::<WebBridgeState>();
        match start_server(app.clone(), port) {
            Ok(runtime) => *bridge.runtime.lock().map_err(lock_error)? = Some(runtime),
            Err(error) => *bridge.last_error.lock().map_err(lock_error)? = Some(error),
        }
    }
    Ok(())
}

pub(crate) fn web_access_status(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<WebAccessStatus, String> {
    let connection = db::open_database(state)?;
    let enabled = db::get_setting(&connection, "web_access_enabled")?.as_deref() == Some("true");
    let port = effective_web_port(
        db::get_setting(&connection, "web_access_port")?
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port >= MIN_WEB_PORT)
            .unwrap_or(DEFAULT_WEB_PORT),
    );
    let bridge = app.state::<WebBridgeState>();
    let available = bridge
        .runtime
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .is_some_and(|runtime| runtime.port == port);
    let error = enabled
        .then(|| {
            bridge
                .last_error
                .lock()
                .map_err(lock_error)
                .map(|value| value.clone())
        })
        .transpose()?
        .flatten();
    Ok(WebAccessStatus {
        enabled,
        port,
        available,
        error,
    })
}

#[tauri::command]
pub(crate) async fn invoke_local(
    app: tauri::AppHandle,
    command: String,
    args: Value,
) -> Result<Value, String> {
    dispatch_command(app, &command, args).await
}

pub(crate) fn set_web_access_settings(
    app: tauri::AppHandle,
    enabled: bool,
    port: u16,
) -> Result<WebAccessStatus, String> {
    let port = effective_web_port(port);
    if port < MIN_WEB_PORT {
        return Err(format!("Web 访问端口必须在 {MIN_WEB_PORT} 到 65535 之间。"));
    }
    let bridge = app.state::<WebBridgeState>();
    let _guard = bridge.update_lock.lock().map_err(lock_error)?;
    let state = app.state::<AppState>();
    let current = web_access_status(&app, &state)?;
    let needs_server = enabled && (!current.available || current.port != port);
    let next_runtime = needs_server
        .then(|| start_server(app.clone(), port))
        .transpose()?;

    let mut connection = db::open_database(&state)?;
    let save_result = if cfg!(debug_assertions) {
        db::set_setting(
            &connection,
            "web_access_enabled",
            enabled.to_string().as_str(),
        )
    } else {
        db::set_web_access_settings(&mut connection, enabled, port)
    };
    if let Err(error) = save_result {
        if let Some(runtime) = next_runtime {
            runtime.stop();
        }
        return Err(error);
    }

    let old_runtime = {
        let mut runtime = bridge.runtime.lock().map_err(lock_error)?;
        if enabled {
            next_runtime.and_then(|next| runtime.replace(next))
        } else {
            runtime.take()
        }
    };
    if let Some(runtime) = old_runtime {
        runtime.stop();
    }
    *bridge.last_error.lock().map_err(lock_error)? = None;
    tray::refresh_tray(&app)?;
    web_access_status(&app, &state)
}

pub(crate) fn record_oauth_progress(
    app: &tauri::AppHandle,
    progress: OAuthProgress,
    pending: bool,
) {
    if let Some(bridge) = app.try_state::<WebBridgeState>() {
        if let Ok(mut snapshot) = bridge.oauth_progress.lock() {
            snapshot.sequence = snapshot.sequence.saturating_add(1);
            snapshot.pending = pending;
            snapshot.progress = Some(progress);
        }
    }
}

fn oauth_progress_snapshot(app: &tauri::AppHandle) -> Result<OAuthProgressSnapshot, String> {
    app.state::<WebBridgeState>()
        .oauth_progress
        .lock()
        .map(|snapshot| snapshot.clone())
        .map_err(lock_error)
}

pub(crate) fn browser_url(app: &tauri::AppHandle) -> Option<String> {
    let state = app.state::<AppState>();
    let status = web_access_status(app, &state).ok()?;
    if !status.available {
        return None;
    }
    Some(if cfg!(debug_assertions) {
        "http://127.0.0.1:5173".to_string()
    } else {
        format!("http://127.0.0.1:{}", status.port)
    })
}

fn open_web_access(app: tauri::AppHandle) -> Result<(), String> {
    let url = browser_url(&app).ok_or_else(|| "Web 访问未运行。".to_string())?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| error.to_string())
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "Web 服务状态锁不可用。".to_string()
}

#[cfg(test)]
mod tests {
    use super::dispatch::{arg, optional};
    use super::server::{
        is_desktop_only_command, is_development_origin, is_local_host, is_production_origin,
    };
    use super::{effective_web_port, DEFAULT_WEB_PORT};
    use serde_json::json;

    #[test]
    fn parses_required_and_optional_arguments() {
        let args = json!({ "profileId": "abc", "alias": null });
        assert_eq!(arg::<String>(&args, "profileId").unwrap(), "abc");
        assert_eq!(optional::<String>(&args, "alias").unwrap(), None);
        assert!(arg::<String>(&args, "missing").is_err());
    }

    #[test]
    fn restricts_origins() {
        assert!(is_development_origin("http://127.0.0.1:5173"));
        assert!(!is_development_origin("http://127.0.0.1:5174"));
        assert!(is_development_origin("http://localhost:5173"));
        assert!(!is_development_origin("https://localhost:5173"));
        assert!(!is_development_origin("http://example.com:5173"));
        assert!(is_production_origin("http://127.0.0.1:11456", 11456));
        assert!(is_production_origin("http://localhost:11456", 11456));
        assert!(!is_production_origin("http://127.0.0.1:11457", 11456));
        assert!(is_local_host("127.0.0.1:11456", 11456));
        assert!(is_local_host("localhost:11456", 11456));
    }

    #[test]
    fn fixes_development_web_port() {
        assert_eq!(effective_web_port(11457), DEFAULT_WEB_PORT);
    }

    #[test]
    fn keeps_web_access_settings_desktop_only() {
        assert!(is_desktop_only_command("set_web_access_settings"));
        assert!(!is_desktop_only_command("set_autostart"));
    }
}
