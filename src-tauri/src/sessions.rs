use super::{codex::auth_path, *};
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
};

const REQUEST_ID: u64 = 1;
const PAGE_SIZE: u32 = 50;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppServerThread {
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
struct AppServerThreadPage {
    data: Vec<AppServerThread>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CodexSessionSummary {
    id: String,
    name: Option<String>,
    preview: String,
    cwd: String,
    source: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CodexSessionPage {
    sessions: Vec<CodexSessionSummary>,
    next_cursor: Option<String>,
}

#[tauri::command]
pub(super) async fn list_codex_sessions(
    state: State<'_, AppState>,
    cursor: Option<String>,
    archived: bool,
    search_term: Option<String>,
) -> Result<CodexSessionPage, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = run_codex_request(
            &state,
            "thread/list",
            json!({
                "limit": PAGE_SIZE,
                "cursor": cursor,
                "archived": archived,
                "searchTerm": search_term.map(|term| term.trim().to_string()).filter(|term| !term.is_empty()),
                "sortKey": "updated_at",
                "sortDirection": "desc"
            }),
        )?;
        let page: AppServerThreadPage =
            serde_json::from_value(result).map_err(|error| format!("Codex 会话响应格式错误：{error}"))?;
        Ok(CodexSessionPage {
            sessions: page
                .data
                .into_iter()
                .map(|thread| CodexSessionSummary {
                    id: thread.id,
                    name: thread.name,
                    preview: thread.preview,
                    cwd: thread.cwd,
                    source: source_name(&thread.source),
                    created_at: thread.created_at.saturating_mul(1000),
                    updated_at: thread.updated_at.saturating_mul(1000),
                })
                .collect(),
            next_cursor: page.next_cursor,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn rename_codex_session(
    state: State<'_, AppState>,
    session_id: String,
    name: String,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("会话名称不能为空。".to_string());
    }
    mutate_session(
        state,
        session_id,
        "thread/name/set",
        json!({ "name": name }),
    )
    .await
}

#[tauri::command]
pub(super) async fn archive_codex_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    mutate_session(state, session_id, "thread/archive", json!({})).await
}

#[tauri::command]
pub(super) async fn restore_codex_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    mutate_session(state, session_id, "thread/unarchive", json!({})).await
}

#[tauri::command]
pub(super) async fn delete_codex_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    mutate_session(state, session_id, "thread/delete", json!({})).await
}

async fn mutate_session(
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

fn run_codex_request(state: &AppState, method: &str, params: Value) -> Result<Value, String> {
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

fn spawn_codex(state: &AppState, codex_home: &Path) -> Result<Child, String> {
    let home = state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home);
    let mut candidates = vec![PathBuf::from("codex")];
    #[cfg(target_os = "macos")]
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
        home.join(".local/bin/codex"),
    ]);
    #[cfg(target_os = "windows")]
    candidates.extend([
        PathBuf::from("codex.exe"),
        PathBuf::from("codex.cmd"),
        home.join("AppData/Roaming/npm/codex.cmd"),
    ]);

    let mut last_error = None;
    for candidate in candidates {
        let is_cmd = candidate
            .extension()
            .is_some_and(|extension| extension == "cmd");
        let mut command = if is_cmd {
            let mut command = Command::new("cmd");
            command.args(["/D", "/C"]).arg(&candidate).arg("app-server");
            command
        } else {
            let mut command = Command::new(&candidate);
            command.arg("app-server");
            command
        };
        match command
            .env("CODEX_HOME", codex_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => return Ok(child),
            Err(error) => last_error = Some(error),
        }
    }
    Err(format!(
        "未找到可用的 Codex CLI，请先安装或更新 Codex。{}",
        last_error
            .map(|error| format!("（{error}）"))
            .unwrap_or_default()
    ))
}

fn decode_response(line: &str) -> Result<Option<Value>, String> {
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

fn source_name(source: &Value) -> String {
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

fn stop_child(child: &mut Child) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_matching_app_server_responses() {
        assert!(
            decode_response(r#"{"method":"thread/started","params":{}}"#)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            decode_response(r#"{"id":1,"result":{"data":[]}}"#).unwrap(),
            Some(json!({ "data": [] }))
        );
        assert_eq!(
            decode_response(r#"{"id":1,"error":{"message":"missing"}}"#).unwrap_err(),
            "Codex app-server 请求失败：missing"
        );
    }

    #[test]
    fn normalizes_session_sources() {
        assert_eq!(source_name(&json!("cli")), "cli");
        assert_eq!(source_name(&json!({ "custom": "desktop" })), "desktop");
        assert_eq!(source_name(&json!({ "subAgent": {} })), "subAgent");
    }
}
