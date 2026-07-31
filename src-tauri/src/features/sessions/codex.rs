use super::{
    commands::{capabilities, non_empty, trimmed_search, PAGE_SIZE},
    types::{SessionPage, SessionSummary},
};
use crate::{
    features::environment::{codex_command, find_codex},
    platform::state::{AccountProduct, AppState},
    products::codex::auth_path,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tauri::State;
use uuid::Uuid;

const REQUEST_ID: u64 = 1;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AppServerThread {
    id: String,
    name: Option<String>,
    preview: String,
    cwd: String,
    source: Value,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AppServerThreadPage {
    data: Vec<AppServerThread>,
    next_cursor: Option<String>,
}

pub(super) fn list_codex_sessions(
    state: &AppState,
    cursor: Option<String>,
    archived: bool,
    search_term: Option<String>,
) -> Result<SessionPage, String> {
    let result = run_codex_request(
        state,
        "thread/list",
        json!({
            "limit": PAGE_SIZE,
            "cursor": cursor,
            "archived": archived,
            "searchTerm": trimmed_search(search_term),
            "sortKey": "updated_at",
            "sortDirection": "desc"
        }),
    )?;
    let page: AppServerThreadPage = serde_json::from_value(result)
        .map_err(|error| format!("Codex 会话响应格式错误：{error}"))?;
    Ok(SessionPage {
        sessions: page
            .data
            .into_iter()
            .map(|thread| SessionSummary {
                id: thread.id,
                name: thread.name,
                preview: thread.preview,
                cwd: non_empty(thread.cwd),
                source: Some(source_name(&thread.source)),
                created_at: Some(thread.created_at.saturating_mul(1000)),
                updated_at: thread.updated_at.saturating_mul(1000),
            })
            .collect(),
        next_cursor: page.next_cursor,
        capabilities: capabilities(AccountProduct::Codex),
    })
}

pub(super) async fn mutate_codex_session(
    state: State<'_, AppState>,
    session_id: String,
    method: &'static str,
    mut params: Value,
) -> Result<(), String> {
    Uuid::parse_str(&session_id).map_err(|_| "会话 ID 格式无效。".to_string())?;
    params["threadId"] = Value::String(session_id);
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_codex_request(&state, method, params).map(drop)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(super) fn run_codex_request(
    state: &AppState,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let codex_home = auth_path(state)?
        .parent()
        .ok_or_else(|| "无法定位 Codex 主目录。".to_string())?
        .to_path_buf();
    let mut child = spawn_codex(state, &codex_home)?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "无法连接 Codex app-server。".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法读取 Codex app-server。".to_string())?;

    for message in [
        json!({
            "method": "initialize",
            "id": 0,
            "params": { "clientInfo": { "name": "cortana", "title": "Cortana", "version": env!("CARGO_PKG_VERSION") } }
        }),
        json!({ "method": "initialized", "params": {} }),
        json!({ "method": method, "id": REQUEST_ID, "params": params }),
    ] {
        writeln!(stdin, "{message}").map_err(|error| {
            stop_child(&mut child);
            format!("无法向 Codex app-server 发送请求：{error}")
        })?;
    }
    stdin.flush().map_err(|error| {
        stop_child(&mut child);
        format!("无法向 Codex app-server 发送请求：{error}")
    })?;

    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + REQUEST_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = match receiver.recv_timeout(remaining) {
            Ok(Ok(line)) => line,
            Ok(Err(error)) => {
                let stderr = stop_child(&mut child);
                return Err(format!("读取 Codex app-server 失败：{error}{stderr}"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                stop_child(&mut child);
                return Err("读取 Codex 会话超时，请稍后重试。".to_string());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let stderr = stop_child(&mut child);
                return Err(format!("Codex app-server 意外退出。{stderr}"));
            }
        };
        match decode_response(&line) {
            Ok(Some(result)) => {
                stop_child(&mut child);
                return Ok(result);
            }
            Ok(None) => {}
            Err(error) => {
                let stderr = stop_child(&mut child);
                return Err(format!("{error}{stderr}"));
            }
        }
    }
}

pub(super) fn spawn_codex(state: &AppState, codex_home: &Path) -> Result<Child, String> {
    let (candidate, _) = find_codex(state)
        .ok_or_else(|| "未找到可用的 Codex CLI，请先安装或更新 Codex。".to_string())?;
    codex_command(&candidate, &["app-server"])
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动 Codex app-server：{error}"))
}

pub(super) fn decode_response(line: &str) -> Result<Option<Value>, String> {
    let message: Value = serde_json::from_str(line)
        .map_err(|error| format!("Codex app-server 返回了无效 JSON：{error}"))?;
    if message.get("id").and_then(Value::as_u64) != Some(REQUEST_ID) {
        return Ok(None);
    }
    if let Some(error) = message.get("error") {
        let text = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("未知错误");
        return Err(format!("Codex app-server 请求失败：{text}"));
    }
    message
        .get("result")
        .cloned()
        .map(Some)
        .ok_or_else(|| "Codex app-server 响应缺少 result。".to_string())
}

pub(super) fn source_name(source: &Value) -> String {
    source
        .as_str()
        .or_else(|| source.get("custom").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| {
            if source.get("subAgent").is_some() {
                "subAgent".to_string()
            } else {
                "unknown".to_string()
            }
        })
}

pub(super) fn stop_child(child: &mut Child) -> String {
    #[cfg(target_os = "windows")]
    let _ = codex_command(
        Path::new("taskkill"),
        &["/PID", &child.id().to_string(), "/T", "/F"],
    )
    .output();
    #[cfg(not(target_os = "windows"))]
    let _ = child.kill();
    let _ = child.wait();
    let mut stderr = String::new();
    if let Some(mut stream) = child.stderr.take() {
        let _ = stream.read_to_string(&mut stderr);
    }
    let stderr = stderr.trim();
    if stderr.is_empty() {
        String::new()
    } else {
        format!(" {stderr}")
    }
}
