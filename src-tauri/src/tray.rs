use super::{accounts::*, antigravity, claude, db::*, grok, *};

pub(super) fn install_tray(app: &tauri::AppHandle) -> Result<(), String> {
    let menu = build_tray_menu(app)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(tauri::include_image!("icons/tray-icon.png"))
        .icon_as_template(true)
        .menu(&menu)
        .tooltip("Cortana")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if id == "show" {
                show_main_window(app);
            } else if id == "open-web" {
                if let Some(url) = local_web::browser_url(app) {
                    let _ = app.opener().open_url(url, None::<&str>);
                }
            } else if id == "quit" {
                app.exit(0);
            } else if let Some(value) = id.strip_prefix("switch:") {
                let Some((product, profile_id)) = value.split_once(':') else {
                    return;
                };
                if product == "antigravity" {
                    let app = app.clone();
                    let profile_id = profile_id.to_string();
                    tauri::async_runtime::spawn_blocking(move || {
                        let result = antigravity::switch_profile(
                            &app.state::<AppState>(),
                            &profile_id,
                            false,
                        );
                        if result.is_err() {
                            show_main_window(&app);
                        } else {
                            let _ = refresh_tray(&app);
                        }
                    });
                    return;
                }
                if product == "claude" {
                    let app = app.clone();
                    let profile_id = profile_id.to_string();
                    tauri::async_runtime::spawn_blocking(move || {
                        let result =
                            claude::switch_profile(&app.state::<AppState>(), &profile_id, false);
                        if result.is_err() {
                            show_main_window(&app);
                        } else {
                            let _ = refresh_tray(&app);
                        }
                    });
                    return;
                }
                if product == "grok" {
                    let app = app.clone();
                    let profile_id = profile_id.to_string();
                    tauri::async_runtime::spawn_blocking(move || {
                        let result =
                            grok::switch_profile(&app.state::<AppState>(), &profile_id, false);
                        if result.is_err() {
                            show_main_window(&app);
                        } else {
                            let _ = refresh_tray(&app);
                        }
                    });
                    return;
                }
                if product != "codex" {
                    return;
                }
                let result = switch_profile_internal(&app.state::<AppState>(), profile_id, false);
                if result.is_err() {
                    show_main_window(app);
                } else {
                    let _ = refresh_tray(app);
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn refresh_tray(app: &tauri::AppHandle) -> Result<(), String> {
    let menu = build_tray_menu(app)?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn build_tray_menu(app: &tauri::AppHandle) -> Result<Menu<tauri::Wry>, String> {
    let state = app.state::<AppState>();
    let connection = open_database(&state)?;
    let product = match get_setting(&connection, "active_product")?.as_deref() {
        Some("claude") => AccountProduct::Claude,
        Some("antigravity") => AccountProduct::Antigravity,
        Some("grok") => AccountProduct::Grok,
        _ => AccountProduct::Codex,
    };
    drop(connection);
    let profiles = match product {
        AccountProduct::Codex => app_status(app, &state)?.profiles,
        AccountProduct::Claude => claude::app_status(app, &state)?.profiles,
        AccountProduct::Antigravity => antigravity::app_status(app, &state)?.profiles,
        AccountProduct::Grok => grok::app_status(app, &state)?.profiles,
    };
    let menu = Menu::new(app).map_err(|error| error.to_string())?;
    let current_label = profiles
        .iter()
        .find(|profile| profile.is_active)
        .map(|profile| format!("当前：{}", profile.alias))
        .unwrap_or_else(|| "当前：未选择账户".to_string());
    let current = MenuItem::with_id(app, "current", current_label, false, None::<&str>)
        .map_err(|error| error.to_string())?;
    menu.append(&current).map_err(|error| error.to_string())?;
    menu.append(&PredefinedMenuItem::separator(app).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    for profile in profiles {
        let label = if profile.email.is_empty() {
            profile.alias.clone()
        } else {
            format!("{} ({})", profile.alias, profile.email)
        };
        let item = MenuItem::with_id(
            app,
            format!("switch:{}:{}", product.as_str(), profile.id),
            label,
            !profile.is_active,
            None::<&str>,
        )
        .map_err(|error| error.to_string())?;
        menu.append(&item).map_err(|error| error.to_string())?;
    }
    menu.append(&PredefinedMenuItem::separator(app).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let show = MenuItem::with_id(app, "show", "打开账户管理", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let web_status = local_web::web_access_status(app, &state)?;
    let (web_label, web_enabled) = if !web_status.enabled {
        ("浏览器访问未启用", false)
    } else if web_status.available {
        ("在浏览器中打开", true)
    } else {
        ("浏览器访问不可用", false)
    };
    let open_web = MenuItem::with_id(app, "open-web", web_label, web_enabled, None::<&str>)
        .map_err(|error| error.to_string())?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    menu.append(&show).map_err(|error| error.to_string())?;
    menu.append(&open_web).map_err(|error| error.to_string())?;
    menu.append(&quit).map_err(|error| error.to_string())?;
    Ok(menu)
}

pub(super) fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
