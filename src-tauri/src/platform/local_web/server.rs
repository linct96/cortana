use super::{dispatch_command, DEVELOPMENT_WEB_PORT};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{io::Read, sync::Arc, thread};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
const WORKER_COUNT: usize = 4;

pub(super) struct RunningServer {
    pub(super) port: u16,
    server: Arc<Server>,
    running: Arc<AtomicBool>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl RunningServer {
    pub(super) fn stop(self) {
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

pub(super) fn start_server(app: tauri::AppHandle, port: u16) -> Result<RunningServer, String> {
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
        workers.push(thread::spawn(move || {
            while running.load(Ordering::Acquire) {
                match server.recv() {
                    Ok(request) => handle_request(request, &app, port),
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

pub(super) fn effective_web_port(port: u16) -> u16 {
    if cfg!(debug_assertions) {
        DEVELOPMENT_WEB_PORT
    } else {
        port
    }
}

fn handle_request(mut request: Request, app: &tauri::AppHandle, port: u16) {
    let host = header(&request, "Host").unwrap_or_default();
    if !is_local_host(&host, port) {
        respond_text(request, 403, "请求来源无效。", &[]);
        return;
    }

    if request.url() == "/api/invoke" {
        let origin = header(&request, "Origin").unwrap_or_default();
        if !(is_production_origin(&origin, port)
            || cfg!(debug_assertions) && is_development_origin(&origin))
        {
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
        if is_desktop_only_command(&invoke.command) {
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

pub(super) fn is_development_origin(origin: &str) -> bool {
    matches!(origin, "http://127.0.0.1:5173" | "http://localhost:5173")
}

pub(super) fn is_production_origin(origin: &str, port: u16) -> bool {
    origin == format!("http://127.0.0.1:{port}") || origin == format!("http://localhost:{port}")
}

pub(super) fn is_local_host(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

pub(super) fn is_desktop_only_command(command: &str) -> bool {
    command == "set_web_access_settings"
}

fn cors_headers(origin: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Access-Control-Allow-Origin", origin.to_string()),
        ("Vary", "Origin".to_string()),
        ("Access-Control-Allow-Methods", "POST, OPTIONS".to_string()),
        ("Access-Control-Allow-Headers", "Content-Type".to_string()),
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
