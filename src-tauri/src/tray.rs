use super::*;

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
    let menu = Menu::new(app).map_err(|error| error.to_string())?;
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
