use super::{accounts::*, codex::*, db::*, *};

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
            } else if id == "quit" {
                app.exit(0);
            } else if let Some(profile_id) = id.strip_prefix("switch:") {
                let state = app.state::<AppState>();
                if switch_profile_internal(&state, profile_id, false).is_err() {
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
    let configured_active_id = get_setting(&connection, "active_profile_id")?;
    let path = auth_path(&state)?;
    let auth_state = resolve_auth_state(&connection, configured_active_id.as_deref(), &path)?;
    let active_id = managed_active_profile_id(configured_active_id, &auth_state);
    let profiles = list_profiles(&connection, active_id.as_deref())?;
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
            format!("switch:{}", profile.id),
            label,
            true,
            None::<&str>,
        )
        .map_err(|error| error.to_string())?;
        menu.append(&item).map_err(|error| error.to_string())?;
    }
    menu.append(&PredefinedMenuItem::separator(app).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let show = MenuItem::with_id(app, "show", "打开账户管理", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    menu.append(&show).map_err(|error| error.to_string())?;
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
