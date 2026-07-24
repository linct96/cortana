use super::{
    accounts, agents, analytics, billing, codex, db, env, oauth, sessions, state::*, tray, *,
};
use serde::de::DeserializeOwned;
use std::sync::atomic::{AtomicBool, Ordering};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

pub(super) const DEFAULT_WEB_PORT: u16 = 11456;
const MIN_WEB_PORT: u16 = 1024;
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
const WORKER_COUNT: usize = 4;

pub(super) struct WebBridgeState {
    token: String,
    runtime: Mutex<Option<RunningServer>>,
    last_error: Mutex<Option<String>>,
    update_lock: Mutex<()>,
    oauth_progress: Mutex<OAuthProgressSnapshot>,
}

struct RunningServer {
    port: u16,
    server: Arc<Server>,
    running: Arc<AtomicBool>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl RunningServer {
    fn stop(self) {
        self.running.store(false, Ordering::Release);
        for _ in &self.workers {
            self.server.unblock();
        }
        for worker in self.workers {
            let _ = worker.join();
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvokeRequest {
    command: String,
    #[serde(default = "empty_args")]
    args: Value,
}

fn empty_args() -> Value {
    json!({})
}

pub(super) fn initialize(app: &tauri::AppHandle) -> Result<(), String> {
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
    let port = stored_port
        .as_deref()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port >= MIN_WEB_PORT)
        .unwrap_or(DEFAULT_WEB_PORT);
    if stored_port.as_deref() != Some(port.to_string().as_str()) {
        db::set_web_access_settings(&mut connection, enabled, port)?;
    }
    let token = match db::get_setting(&connection, "web_access_token")? {
        Some(token) if !token.is_empty() => token,
        _ => {
            let token = oauth::random_urlsafe(32);
            db::set_setting(&connection, "web_access_token", &token)?;
            token
        }
    };
    drop(connection);

    app.manage(WebBridgeState {
        token,
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
        match start_server(app.clone(), port, bridge.token.clone()) {
            Ok(runtime) => *bridge.runtime.lock().map_err(lock_error)? = Some(runtime),
            Err(error) => *bridge.last_error.lock().map_err(lock_error)? = Some(error),
        }
    }
    Ok(())
}

pub(super) fn web_access_status(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<WebAccessStatus, String> {
    let connection = db::open_database(state)?;
    let enabled = db::get_setting(&connection, "web_access_enabled")?.as_deref() == Some("true");
    let port = db::get_setting(&connection, "web_access_port")?
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port >= MIN_WEB_PORT)
        .unwrap_or(DEFAULT_WEB_PORT);
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
pub(super) async fn invoke_local(
    app: tauri::AppHandle,
    command: String,
    #[allow(unused_variables)] args: Value,
) -> Result<Value, String> {
    dispatch_command(app, &command, args).await
}

async fn dispatch_command(
    app: tauri::AppHandle,
    command: &str,
    args: Value,
) -> Result<Value, String> {
    macro_rules! result {
        ($value:expr) => {
            serde_json::to_value($value?).map_err(|error| error.to_string())
        };
    }
    match command {
        "get_app_status" => result!(
            accounts::get_app_status(app.clone(), app.state(), optional(&args, "product")?).await
        ),
        "get_terminal_app" => result!(env::get_terminal_app(app.state())),
        "set_terminal_app" => result!(env::set_terminal_app(
            app.state(),
            arg(&args, "terminalApp")?
        )),
        "switch_profile" => result!(
            accounts::switch_profile(
                app.clone(),
                app.state(),
                arg(&args, "profileId")?,
                arg(&args, "force")?,
                optional(&args, "product")?
            )
            .await
        ),
        "open_codex_cli_with_profile" => result!(
            accounts::open_codex_cli_with_profile(app.state(), arg(&args, "profileId")?).await
        ),
        "start_oauth_add" => result!(oauth::start_oauth_add(
            app.clone(),
            app.state(),
            optional(&args, "alias")?,
            arg(&args, "activate")?,
            optional(&args, "product")?
        )),
        "cancel_oauth_add" => result!(oauth::cancel_oauth_add(app.clone(), app.state())),
        "import_current_profile" => result!(
            accounts::import_current_profile(
                app.clone(),
                app.state(),
                optional(&args, "alias")?,
                optional(&args, "product")?
            )
            .await
        ),
        "import_auth_json" => result!(
            oauth::import_auth_json(
                app.clone(),
                app.state(),
                arg(&args, "authJson")?,
                optional(&args, "alias")?,
                arg(&args, "activate")?
            )
            .await
        ),
        "add_relay_profile" => result!(accounts::add_relay_profile(
            app.clone(),
            app.state(),
            arg(&args, "apiKey")?,
            arg(&args, "apiBaseUrl")?,
            optional(&args, "alias")?,
            arg(&args, "activate")?,
            optional(&args, "product")?
        )),
        "refresh_profile_usage" => {
            result!(accounts::refresh_profile_usage(app.state(), arg(&args, "profileId")?).await)
        }
        "refresh_due_profile_usage" => result!(
            accounts::refresh_due_profile_usage(app.state(), arg(&args, "immediate")?).await
        ),
        "get_usage_refresh_settings" => {
            result!(accounts::get_usage_refresh_settings(app.state()))
        }
        "set_usage_refresh_settings" => result!(accounts::set_usage_refresh_settings(
            app.state(),
            arg(&args, "enabled")?,
            arg(&args, "activeIntervalMinutes")?,
            arg(&args, "inactiveIntervalMinutes")?
        )),
        "get_profile_reset_credits" => result!(
            accounts::get_profile_reset_credits(app.state(), arg(&args, "profileId")?).await
        ),
        "get_profile_auth" => result!(accounts::get_profile_auth(
            app.state(),
            arg(&args, "profileId")?
        )),
        "update_profile" => result!(accounts::update_profile(
            app.clone(),
            app.state(),
            arg(&args, "profileId")?,
            arg(&args, "alias")?,
            optional(&args, "authJson")?,
            optional(&args, "product")?
        )),
        "update_relay_profile" => result!(accounts::update_relay_profile(
            app.clone(),
            app.state(),
            arg(&args, "profileId")?,
            arg(&args, "alias")?,
            optional(&args, "apiKey")?,
            arg(&args, "apiBaseUrl")?,
            optional(&args, "product")?
        )),
        "reorder_profiles" => result!(accounts::reorder_profiles(
            app.clone(),
            app.state(),
            arg(&args, "profileIds")?,
            optional(&args, "product")?
        )),
        "delete_profile" => result!(accounts::delete_profile(
            app.clone(),
            app.state(),
            arg(&args, "profileId")?,
            optional(&args, "product")?
        )),
        "get_active_product" => result!(accounts::get_active_product(app.state())),
        "set_active_product" => result!(accounts::set_active_product(
            app.clone(),
            app.state(),
            arg(&args, "product")?
        )),
        "set_codex_home" => result!(accounts::set_codex_home(
            app.clone(),
            app.state(),
            arg(&args, "codexHome")?
        )),
        "get_agents_status" => result!(agents::get_agents_status(app.state())),
        "create_agents_profile" => result!(agents::create_agents_profile(
            app.state(),
            arg(&args, "name")?,
            arg(&args, "content")?
        )),
        "update_agents_profile" => result!(agents::update_agents_profile(
            app.state(),
            arg(&args, "profileId")?,
            arg(&args, "name")?,
            arg(&args, "content")?
        )),
        "activate_agents_profile" => result!(agents::activate_agents_profile(
            app.state(),
            arg(&args, "profileId")?,
            arg(&args, "force")?
        )),
        "import_current_agents" => result!(agents::import_current_agents(
            app.state(),
            arg(&args, "name")?
        )),
        "delete_agents_profile" => result!(agents::delete_agents_profile(
            app.state(),
            arg(&args, "profileId")?
        )),
        "get_usage_analytics" => {
            result!(
                analytics::get_usage_analytics(
                    app.state(),
                    arg(&args, "product")?,
                    arg(&args, "range")?
                )
                .await
            )
        }
        "list_model_pricing" => result!(billing::list_model_pricing(app.state())),
        "save_model_pricing" => result!(billing::save_model_pricing(
            app.state(),
            arg(&args, "items")?
        )),
        "delete_model_pricing" => result!(billing::delete_model_pricing(
            app.state(),
            arg(&args, "modelId")?
        )),
        "fetch_models_dev_pricing" => result!(billing::fetch_models_dev_pricing().await),
        "get_codex_config" => result!(codex::get_codex_config(app.state())),
        "validate_codex_config" => result!(Ok::<_, String>(codex::validate_codex_config(arg(
            &args, "content"
        )?))),
        "format_codex_config" => result!(codex::format_codex_config(arg(&args, "content")?)),
        "save_codex_config" => result!(codex::save_codex_config(
            app.state(),
            arg(&args, "content")?
        )),
        "get_claude_config" => result!(claude::get_claude_config(app.state())),
        "validate_claude_config" => result!(Ok::<_, String>(claude::validate_claude_config(arg(
            &args, "content"
        )?))),
        "format_claude_config" => result!(claude::format_claude_config(arg(&args, "content")?)),
        "save_claude_config" => result!(claude::save_claude_config(
            app.state(),
            arg(&args, "content")?
        )),
        "get_antigravity_config" => result!(antigravity::get_antigravity_config(app.state())),
        "validate_antigravity_config" => result!(Ok::<_, String>(
            antigravity::validate_antigravity_config(arg(&args, "content")?)
        )),
        "format_antigravity_config" => result!(antigravity::format_antigravity_config(arg(
            &args, "content"
        )?)),
        "save_antigravity_config" => result!(antigravity::save_antigravity_config(
            app.state(),
            arg(&args, "content")?
        )),
        "get_grok_config" => result!(grok::get_grok_config(app.state())),
        "validate_grok_config" => result!(Ok::<_, String>(grok::validate_grok_config(arg(
            &args, "content"
        )?))),
        "format_grok_config" => result!(grok::format_grok_config(arg(&args, "content")?)),
        "save_grok_config" => result!(grok::save_grok_config(app.state(), arg(&args, "content")?)),
        "is_codex_cli_available" => result!(env::is_codex_cli_available(app.state()).await),
        "is_claude_cli_available" => result!(env::is_claude_cli_available(app.state()).await),
        "is_antigravity_cli_available" => {
            result!(env::is_antigravity_cli_available(app.state()).await)
        }
        "is_grok_cli_available" => result!(env::is_grok_cli_available(app.state()).await),
        "get_codex_cli_environment" => result!(env::get_codex_cli_environment(app.state()).await),
        "get_claude_cli_environment" => {
            result!(env::get_claude_cli_environment(app.state()).await)
        }
        "get_antigravity_cli_environment" => {
            result!(env::get_antigravity_cli_environment(app.state()).await)
        }
        "get_grok_cli_environment" => {
            result!(env::get_grok_cli_environment(app.state()).await)
        }
        "list_sessions" => result!(
            sessions::list_sessions(
                app.state(),
                arg(&args, "product")?,
                optional(&args, "cursor")?,
                arg(&args, "archived")?,
                optional(&args, "searchTerm")?
            )
            .await
        ),
        "rename_session" => result!(
            sessions::rename_session(
                app.state(),
                arg(&args, "product")?,
                arg(&args, "sessionId")?,
                arg(&args, "name")?
            )
            .await
        ),
        "archive_session" => {
            result!(
                sessions::archive_session(
                    app.state(),
                    arg(&args, "product")?,
                    arg(&args, "sessionId")?
                )
                .await
            )
        }
        "unarchive_session" => {
            result!(
                sessions::unarchive_session(
                    app.state(),
                    arg(&args, "product")?,
                    arg(&args, "sessionId")?
                )
                .await
            )
        }
        "delete_session" => {
            result!(
                sessions::delete_session(
                    app.state(),
                    arg(&args, "product")?,
                    arg(&args, "sessionId")?
                )
                .await
            )
        }
        "set_autostart" => result!(super::set_autostart(
            app.clone(),
            app.state(),
            arg(&args, "enabled")?
        )),
        "reveal_data_directory" => {
            result!(super::reveal_data_directory(app.clone(), app.state()))
        }
        "open_codex_home" => result!(super::open_codex_home(
            app.clone(),
            arg(&args, "codexHome")?
        )),
        "open_codex_cli_install_page" => {
            result!(super::open_codex_cli_install_page(app.clone()))
        }
        "open_claude_cli_install_page" => {
            result!(super::open_claude_cli_install_page(app.clone()))
        }
        "open_antigravity_cli_install_page" => {
            result!(super::open_antigravity_cli_install_page(app.clone()))
        }
        "open_grok_cli_install_page" => result!(super::open_grok_cli_install_page(app.clone())),
        "open_web_access" => result!(open_web_access(app.clone())),
        "set_web_access_settings" => result!(set_web_access_settings(
            app.clone(),
            arg(&args, "enabled")?,
            arg(&args, "port")?
        )),
        "get_oauth_progress" => result!(Ok::<_, String>(oauth_progress_snapshot(&app)?)),
        _ => Err(format!("未知命令：{command}")),
    }
}

fn arg<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
    args.get(key)
        .cloned()
        .ok_or_else(|| format!("缺少参数：{key}"))
        .and_then(|value| serde_json::from_value(value).map_err(|_| format!("参数格式无效：{key}")))
}

fn optional<T: DeserializeOwned>(args: &Value, key: &str) -> Result<Option<T>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| format!("参数格式无效：{key}")),
    }
}

pub(super) fn set_web_access_settings(
    app: tauri::AppHandle,
    enabled: bool,
    port: u16,
) -> Result<WebAccessStatus, String> {
    if port < MIN_WEB_PORT {
        return Err(format!("Web 访问端口必须在 {MIN_WEB_PORT} 到 65535 之间。"));
    }
    let bridge = app.state::<WebBridgeState>();
    let _guard = bridge.update_lock.lock().map_err(lock_error)?;
    let state = app.state::<AppState>();
    let current = web_access_status(&app, &state)?;
    let needs_server = enabled && (!current.available || current.port != port);
    let next_runtime = needs_server
        .then(|| start_server(app.clone(), port, bridge.token.clone()))
        .transpose()?;

    let mut connection = db::open_database(&state)?;
    if let Err(error) = db::set_web_access_settings(&mut connection, enabled, port) {
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

fn start_server(app: tauri::AppHandle, port: u16, token: String) -> Result<RunningServer, String> {
    let server = Arc::new(
        Server::http(("127.0.0.1", port))
            .map_err(|error| format!("无法监听 127.0.0.1:{port}：{error}"))?,
    );
    let running = Arc::new(AtomicBool::new(true));
    let mut workers = Vec::with_capacity(WORKER_COUNT);
    for _ in 0..WORKER_COUNT {
        let server = server.clone();
        let running = running.clone();
        let app = app.clone();
        let token = token.clone();
        workers.push(thread::spawn(move || {
            while running.load(Ordering::Acquire) {
                match server.recv() {
                    Ok(request) => handle_request(request, &app, port, &token),
                    Err(error) if running.load(Ordering::Acquire) => {
                        eprintln!("Web bridge request failed: {error}");
                        break;
                    }
                    Err(_) => break,
                }
            }
        }));
    }
    Ok(RunningServer {
        port,
        server,
        running,
        workers,
    })
}

fn handle_request(mut request: Request, app: &tauri::AppHandle, port: u16, token: &str) {
    let host = header(&request, "Host").unwrap_or_default();
    if !is_local_host(&host, port) {
        respond_text(request, 403, "请求来源无效。", &[]);
        return;
    }

    if request.url() == "/api/invoke" {
        let origin = header(&request, "Origin").unwrap_or_default();
        let development = cfg!(debug_assertions) && is_development_origin(&origin);
        if !is_production_origin(&origin, port) && !development {
            respond_text(request, 403, "请求来源无效。", &[]);
            return;
        }
        let cors = cors_headers(&origin);

        if request.method() == &Method::Options {
            respond_text(request, 204, "", &cors);
            return;
        }
        if request.method() != &Method::Post {
            respond_text(request, 405, "仅支持 POST 请求。", &cors);
            return;
        }
        if !is_authorized(
            header(&request, "Authorization").as_deref(),
            token,
            development,
        ) {
            respond_text(request, 401, "Web 访问授权无效。", &cors);
            return;
        }
        if header(&request, "Content-Type")
            .as_deref()
            .and_then(|value| value.split(';').next())
            != Some("application/json")
        {
            respond_text(request, 415, "请求必须使用 application/json。", &cors);
            return;
        }
        if request
            .body_length()
            .is_some_and(|size| size > MAX_REQUEST_BYTES)
        {
            respond_text(request, 413, "请求数据过大。", &cors);
            return;
        }
        let mut body = Vec::new();
        if request
            .as_reader()
            .take((MAX_REQUEST_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .is_err()
        {
            respond_text(request, 400, "无法读取请求数据。", &cors);
            return;
        }
        if body.len() > MAX_REQUEST_BYTES {
            respond_text(request, 413, "请求数据过大。", &cors);
            return;
        }
        let invoke = match serde_json::from_slice::<InvokeRequest>(&body) {
            Ok(invoke) => invoke,
            Err(_) => {
                respond_text(request, 400, "请求 JSON 格式无效。", &cors);
                return;
            }
        };
        if invoke.command == "set_web_access_settings" {
            respond_text(request, 403, "仅可在桌面应用中修改 Web 访问设置。", &cors);
            return;
        }
        let response = tauri::async_runtime::block_on(dispatch_command(
            app.clone(),
            &invoke.command,
            invoke.args,
        ));
        match response {
            Ok(value) => respond_json(request, 200, value, &cors),
            Err(error) => respond_text(request, 400, &error, &cors),
        }
        return;
    }

    if cfg!(debug_assertions) {
        respond_text(request, 404, "开发环境请访问 http://127.0.0.1:5173。", &[]);
        return;
    }
    if request.method() != &Method::Get {
        respond_text(request, 405, "仅支持 GET 请求。", &[]);
        return;
    }
    let path = request.url().split('?').next().unwrap_or("/").to_string();
    match app.asset_resolver().get(path) {
        Some(asset) => {
            let mut headers = vec![
                ("Content-Type", asset.mime_type),
                ("Cache-Control", "no-cache".to_string()),
                ("X-Content-Type-Options", "nosniff".to_string()),
                ("Referrer-Policy", "no-referrer".to_string()),
            ];
            if let Some(csp) = asset.csp_header {
                headers.push(("Content-Security-Policy", csp));
            }
            respond_data(request, 200, asset.bytes, &headers);
        }
        None => respond_text(request, 404, "页面不存在。", &[]),
    }
}

fn header(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.to_string().eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str().to_string())
}

fn is_development_origin(origin: &str) -> bool {
    origin
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| origin.strip_prefix("http://localhost:"))
        .is_some_and(|port| port.parse::<u16>().is_ok())
}

fn is_production_origin(origin: &str, port: u16) -> bool {
    origin == format!("http://127.0.0.1:{port}") || origin == format!("http://localhost:{port}")
}

fn is_local_host(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

fn is_authorized(authorization: Option<&str>, token: &str, development: bool) -> bool {
    development || authorization == Some(format!("Bearer {token}").as_str())
}

fn cors_headers(origin: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Access-Control-Allow-Origin", origin.to_string()),
        ("Vary", "Origin".to_string()),
        ("Access-Control-Allow-Methods", "POST, OPTIONS".to_string()),
        (
            "Access-Control-Allow-Headers",
            "Content-Type, Authorization".to_string(),
        ),
    ]
}

fn respond_json(request: Request, status: u16, value: Value, extra: &[(&str, String)]) {
    match serde_json::to_vec(&value) {
        Ok(body) => {
            let mut headers = vec![
                (
                    "Content-Type",
                    "application/json; charset=utf-8".to_string(),
                ),
                ("Cache-Control", "no-store".to_string()),
            ];
            headers.extend(extra.iter().cloned());
            respond_data(request, status, body, &headers);
        }
        Err(_) => respond_text(request, 500, "响应序列化失败。", extra),
    }
}

fn respond_text(request: Request, status: u16, body: &str, extra: &[(&str, String)]) {
    let mut headers = vec![
        ("Content-Type", "text/plain; charset=utf-8".to_string()),
        ("Cache-Control", "no-store".to_string()),
    ];
    headers.extend(extra.iter().cloned());
    respond_data(request, status, body.as_bytes().to_vec(), &headers);
}

fn respond_data(request: Request, status: u16, body: Vec<u8>, headers: &[(&str, String)]) {
    let mut response = Response::from_data(body).with_status_code(StatusCode(status));
    for (name, value) in headers {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response.add_header(header);
        }
    }
    let _ = request.respond(response);
}

pub(super) fn record_oauth_progress(
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

pub(super) fn browser_url(app: &tauri::AppHandle) -> Option<String> {
    let state = app.state::<AppState>();
    let status = web_access_status(app, &state).ok()?;
    status.available.then(|| {
        if cfg!(debug_assertions) {
            format!("http://127.0.0.1:5173/?port={}#/", status.port)
        } else {
            let token = &app.state::<WebBridgeState>().token;
            format!("http://127.0.0.1:{}/?token={token}#/", status.port)
        }
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
    use super::*;

    #[test]
    fn parses_required_and_optional_arguments() {
        let args = json!({ "profileId": "abc", "alias": null });
        assert_eq!(arg::<String>(&args, "profileId").unwrap(), "abc");
        assert_eq!(optional::<String>(&args, "alias").unwrap(), None);
        assert!(arg::<String>(&args, "missing").is_err());
    }

    #[test]
    fn restricts_origins_and_tokens() {
        assert!(is_development_origin("http://127.0.0.1:5173"));
        assert!(is_development_origin("http://localhost:5174"));
        assert!(!is_development_origin("https://localhost:5173"));
        assert!(!is_development_origin("http://example.com:5173"));
        assert!(is_production_origin("http://127.0.0.1:11456", 11456));
        assert!(is_production_origin("http://localhost:11456", 11456));
        assert!(!is_production_origin("http://127.0.0.1:11457", 11456));
        assert!(is_local_host("localhost:11456", 11456));
        assert!(is_authorized(Some("Bearer secret"), "secret", false));
        assert!(!is_authorized(Some("Bearer wrong"), "secret", false));
        assert!(is_authorized(None, "secret", true));
    }
}
