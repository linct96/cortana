use super::{
    codex::auth_path,
    env::{codex_candidates, codex_command, grok_candidates, grok_home},
    *,
};
use chrono::DateTime;
use rusqlite::OpenFlags;
use std::{
    io::{BufRead, BufReader},
    process::{Child, Stdio},
    sync::mpsc,
    time::UNIX_EPOCH,
};

const REQUEST_ID: u64 = 1;
const PAGE_SIZE: usize = 50;
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

#[derive(Debug, Deserialize)]
struct GrokSessionInfo {
    id: String,
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GrokSessionFile {
    info: GrokSessionInfo,
    session_summary: Option<String>,
    generated_title: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionSummary {
    id: String,
    name: Option<String>,
    preview: String,
    cwd: Option<String>,
    source: Option<String>,
    created_at: Option<i64>,
    updated_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionCapabilities {
    supports_archived: bool,
    can_rename: bool,
    can_archive: bool,
    can_delete: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionPage {
    sessions: Vec<SessionSummary>,
    next_cursor: Option<String>,
    capabilities: SessionCapabilities,
}

#[tauri::command]
pub(super) async fn list_sessions(
    state: State<'_, AppState>,
    product: AccountProduct,
    cursor: Option<String>,
    archived: bool,
    search_term: Option<String>,
) -> Result<SessionPage, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match product {
        AccountProduct::Codex => list_codex_sessions(&state, cursor, archived, search_term),
        AccountProduct::Claude => {
            reject_archived(archived, product)?;
            paginate_sessions(
                list_claude_sessions(&state)?,
                cursor,
                search_term,
                capabilities(product),
            )
        }
        AccountProduct::Grok => {
            reject_archived(archived, product)?;
            paginate_sessions(
                list_grok_sessions(&state)?,
                cursor,
                search_term,
                capabilities(product),
            )
        }
        AccountProduct::Antigravity => {
            reject_archived(archived, product)?;
            paginate_sessions(
                list_antigravity_sessions(&state)?,
                cursor,
                search_term,
                capabilities(product),
            )
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn rename_session(
    state: State<'_, AppState>,
    product: AccountProduct,
    session_id: String,
    name: String,
) -> Result<(), String> {
    if product != AccountProduct::Codex {
        return Err(unsupported_action(product, "重命名"));
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("会话名称不能为空。".to_string());
    }
    mutate_codex_session(
        state,
        session_id,
        "thread/name/set",
        json!({ "name": name }),
    )
    .await
}

#[tauri::command]
pub(super) async fn archive_session(
    state: State<'_, AppState>,
    product: AccountProduct,
    session_id: String,
) -> Result<(), String> {
    if product != AccountProduct::Codex {
        return Err(unsupported_action(product, "归档"));
    }
    mutate_codex_session(state, session_id, "thread/archive", json!({})).await
}

#[tauri::command]
pub(super) async fn unarchive_session(
    state: State<'_, AppState>,
    product: AccountProduct,
    session_id: String,
) -> Result<(), String> {
    if product != AccountProduct::Codex {
        return Err(unsupported_action(product, "恢复"));
    }
    mutate_codex_session(state, session_id, "thread/unarchive", json!({})).await
}

#[tauri::command]
pub(super) async fn delete_session(
    state: State<'_, AppState>,
    product: AccountProduct,
    session_id: String,
) -> Result<(), String> {
    Uuid::parse_str(&session_id).map_err(|_| "会话 ID 格式无效。".to_string())?;
    match product {
        AccountProduct::Codex => {
            mutate_codex_session(state, session_id, "thread/delete", json!({})).await
        }
        AccountProduct::Grok => {
            let state = state.inner().clone();
            tauri::async_runtime::spawn_blocking(move || delete_grok_session(&state, &session_id))
                .await
                .map_err(|error| error.to_string())?
        }
        _ => Err(unsupported_action(product, "删除")),
    }
}

fn capabilities(product: AccountProduct) -> SessionCapabilities {
    match product {
        AccountProduct::Codex => SessionCapabilities {
            supports_archived: true,
            can_rename: true,
            can_archive: true,
            can_delete: true,
        },
        AccountProduct::Grok => SessionCapabilities {
            supports_archived: false,
            can_rename: false,
            can_archive: false,
            can_delete: true,
        },
        AccountProduct::Claude | AccountProduct::Antigravity => SessionCapabilities {
            supports_archived: false,
            can_rename: false,
            can_archive: false,
            can_delete: false,
        },
    }
}

fn reject_archived(archived: bool, product: AccountProduct) -> Result<(), String> {
    if archived {
        Err(unsupported_action(product, "查看归档会话"))
    } else {
        Ok(())
    }
}

fn unsupported_action(product: AccountProduct, action: &str) -> String {
    format!("{} 暂不支持会话{action}。", product.display_name())
}

fn list_codex_sessions(
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

fn list_claude_sessions(state: &AppState) -> Result<Vec<SessionSummary>, String> {
    let projects = claude_config_dir(state).join("projects");
    if !projects.is_dir() {
        return Ok(Vec::new());
    }

    let mut file_count = 0;
    let mut sessions = Vec::new();
    for project in read_directories(&projects, "Claude 会话目录")? {
        let entries = match fs::read_dir(&project) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                continue;
            }
            file_count += 1;
            if let Some(session) = parse_claude_session(&path) {
                sessions.push(session);
            }
        }
    }
    if file_count > 0 && sessions.is_empty() {
        return Err("Claude 会话文件格式不兼容，请更新 Cortana 后重试。".to_string());
    }
    Ok(sessions)
}

fn parse_claude_session(path: &Path) -> Option<SessionSummary> {
    let id = path.file_stem()?.to_str()?.to_string();
    Uuid::parse_str(&id).ok()?;
    let file = fs::File::open(path).ok()?;
    let modified_at = file_modified_at(path);
    let mut custom_title = None;
    let mut agent_name = None;
    let mut ai_title = None;
    let mut last_prompt = None;
    let mut first_prompt = None;
    let mut cwd = None;
    let mut created_at = None;
    let mut updated_at = None;
    let mut recognized = false;

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(kind) = record.get("type").and_then(Value::as_str) else {
            continue;
        };
        if cwd.is_none() {
            cwd = record
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(|value| non_empty(value.to_string()));
        }
        if let Some(timestamp) = record
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
        {
            created_at = Some(created_at.map_or(timestamp, |current: i64| current.min(timestamp)));
            updated_at = Some(updated_at.map_or(timestamp, |current: i64| current.max(timestamp)));
        }

        match kind {
            "custom-title" => {
                recognized = true;
                custom_title = string_field(&record, &["customTitle", "title"]);
            }
            "agent-name" => {
                recognized = true;
                agent_name = string_field(&record, &["agentName"]);
            }
            "ai-title" => {
                recognized = true;
                ai_title = string_field(&record, &["aiTitle"]);
            }
            "last-prompt" => {
                recognized = true;
                if let Some(prompt) = string_field(&record, &["lastPrompt"]).and_then(usable_prompt)
                {
                    last_prompt = Some(prompt);
                }
            }
            "user"
                if !record
                    .get("isSidechain")
                    .and_then(Value::as_bool)
                    .unwrap_or(false) =>
            {
                recognized = true;
                if let Some(prompt) = record
                    .pointer("/message/content")
                    .and_then(message_text)
                    .and_then(non_empty)
                    .and_then(usable_prompt)
                {
                    if first_prompt.is_none() {
                        first_prompt = Some(prompt.clone());
                    }
                    last_prompt = Some(prompt);
                }
            }
            "assistant" | "mode" | "permission-mode" => {
                recognized = true;
            }
            _ => {}
        }
    }

    if !recognized {
        return None;
    }
    let name = custom_title.or(agent_name).or(ai_title).map(compact_text);
    let preview = compact_text(last_prompt.or(first_prompt).unwrap_or_default());
    if name.is_none() && preview.is_empty() {
        return None;
    }
    Some(SessionSummary {
        id,
        name,
        preview,
        cwd,
        source: Some("cli".to_string()),
        created_at,
        updated_at: updated_at.unwrap_or(modified_at),
    })
}

fn list_grok_sessions(state: &AppState) -> Result<Vec<SessionSummary>, String> {
    let root = grok_home(state).join("sessions");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut sessions = Vec::new();
    for workspace in read_directories(&root, "Grok 会话目录")? {
        for session_dir in read_directories(&workspace, "Grok 会话目录").unwrap_or_default() {
            let path = session_dir.join("summary.json");
            if !path.is_file() {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(summary) = serde_json::from_str::<GrokSessionFile>(&content) else {
                continue;
            };
            if let Some(session) = grok_session_summary(summary, file_modified_at(&path)) {
                sessions.push(session);
            }
        }
    }
    Ok(sessions)
}

fn grok_session_summary(summary: GrokSessionFile, modified_at: i64) -> Option<SessionSummary> {
    Uuid::parse_str(&summary.info.id).ok()?;
    let preview = compact_text(summary.session_summary.unwrap_or_default());
    Some(SessionSummary {
        id: summary.info.id,
        name: summary
            .generated_title
            .and_then(non_empty)
            .map(compact_text),
        preview,
        cwd: summary.info.cwd.and_then(non_empty),
        source: Some("cli".to_string()),
        created_at: summary.created_at.as_deref().and_then(parse_timestamp),
        updated_at: summary
            .updated_at
            .as_deref()
            .and_then(parse_timestamp)
            .unwrap_or(modified_at),
    })
}

fn delete_grok_session(state: &AppState, session_id: &str) -> Result<(), String> {
    let mut last_error = None;
    for candidate in grok_candidates(state) {
        match codex_command(&candidate, &["sessions", "delete", session_id])
            .env("GROK_HOME", grok_home(state))
            .output()
        {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                let message = String::from_utf8_lossy(if output.stderr.is_empty() {
                    &output.stdout
                } else {
                    &output.stderr
                });
                return Err(format!("Grok 会话删除失败：{}", message.trim()));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(format!(
        "未找到可用的 Grok CLI，请先安装或更新 Grok。{}",
        last_error
            .map(|error| format!("（{error}）"))
            .unwrap_or_default()
    ))
}

fn list_antigravity_sessions(state: &AppState) -> Result<Vec<SessionSummary>, String> {
    let path = user_home(state).join(".gemini/antigravity-cli/conversation_summaries.db");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    list_antigravity_sessions_at(&path)
}

fn list_antigravity_sessions_at(path: &Path) -> Result<Vec<SessionSummary>, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("无法只读打开 Antigravity 会话索引：{error}"))?;
    let mut statement = connection
        .prepare(
            "SELECT conversation_id, COALESCE(title, ''), COALESCE(preview, ''),
                    last_modified_time, COALESCE(workspace_uris, '[]'), COALESCE(source, '')
             FROM conversation_summaries
             WHERE nesting_depth = 0 AND killed = 0",
        )
        .map_err(|error| format!("Antigravity 会话索引格式不兼容：{error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|error| format!("无法查询 Antigravity 会话索引：{error}"))?;

    let mut sessions = Vec::new();
    for row in rows.flatten() {
        let Some(updated_at) = parse_antigravity_timestamp(&row.3).filter(|value| *value > 0)
        else {
            continue;
        };
        sessions.push(SessionSummary {
            id: row.0,
            name: non_empty(row.1).map(compact_text),
            preview: compact_text(row.2),
            cwd: workspace_path(&row.4),
            source: non_empty(row.5).or_else(|| Some("cli".to_string())),
            created_at: None,
            updated_at,
        });
    }
    Ok(sessions)
}

fn paginate_sessions(
    mut sessions: Vec<SessionSummary>,
    cursor: Option<String>,
    search_term: Option<String>,
    capabilities: SessionCapabilities,
) -> Result<SessionPage, String> {
    if let Some(search) = trimmed_search(search_term).map(|value| value.to_lowercase()) {
        sessions.retain(|session| {
            session
                .name
                .as_deref()
                .unwrap_or_default()
                .to_lowercase()
                .contains(&search)
                || session.preview.to_lowercase().contains(&search)
        });
    }
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let offset = cursor
        .as_deref()
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| "会话游标无效。".to_string())?;
    let end = offset.saturating_add(PAGE_SIZE).min(sessions.len());
    let page = if offset < sessions.len() {
        sessions[offset..end].to_vec()
    } else {
        Vec::new()
    };
    Ok(SessionPage {
        sessions: page,
        next_cursor: (end < sessions.len()).then(|| end.to_string()),
        capabilities,
    })
}

fn claude_config_dir(state: &AppState) -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home(state).join(".claude"))
}

fn user_home(state: &AppState) -> &Path {
    state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home)
}

fn read_directories(root: &Path, label: &str) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(root).map_err(|error| format!("无法读取{label}：{error}"))?;
    Ok(entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect())
}

fn string_field(record: &Value, fields: &[&str]) -> Option<String> {
    fields
        .iter()
        .find_map(|field| record.get(*field).and_then(Value::as_str))
        .map(str::to_string)
        .and_then(non_empty)
}

fn message_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let parts = content.as_array()?.iter().filter_map(|part| {
        (part.get("type").and_then(Value::as_str) == Some("text"))
            .then(|| part.get("text").and_then(Value::as_str))
            .flatten()
    });
    let text = parts.collect::<Vec<_>>().join(" ");
    non_empty(text)
}

fn compact_text(value: String) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = value.chars();
    let compact = chars.by_ref().take(500).collect::<String>();
    if chars.next().is_some() {
        format!("{compact}...")
    } else {
        compact
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

fn usable_prompt(value: String) -> Option<String> {
    let value = value.trim();
    (!value.starts_with("<local-command-") && !value.starts_with("<system-reminder>"))
        .then(|| value.to_string())
}

fn trimmed_search(value: Option<String>) -> Option<String> {
    value.and_then(non_empty)
}

fn parse_timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn parse_antigravity_timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f%:z")
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn file_modified_at(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn workspace_path(value: &str) -> Option<String> {
    let uri = serde_json::from_str::<Vec<String>>(value)
        .ok()?
        .into_iter()
        .next()?;
    Url::parse(&uri)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .map(|path| path.display().to_string())
}

async fn mutate_codex_session(
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
    let candidates = codex_candidates(state);
    let mut last_error = None;
    for candidate in candidates {
        match codex_command(&candidate, &["app-server"])
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
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
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
    fn parses_claude_session_with_malformed_lines() {
        let root = std::env::temp_dir().join(format!("cortana-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let id = Uuid::new_v4().to_string();
        let path = root.join(format!("{id}.jsonl"));
        fs::write(
            &path,
            concat!(
                "invalid\n",
                "{\"type\":\"user\",\"message\":{\"content\":\"<local-command-caveat>ignore</local-command-caveat>\"}}\n",
                "{\"type\":\"user\",\"message\":{\"content\":\"first prompt\"},\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-07-01T00:00:00Z\"}\n",
                "{\"type\":\"ai-title\",\"aiTitle\":\"Generated title\"}\n",
                "{\"type\":\"last-prompt\",\"lastPrompt\":\"last prompt\"}\n"
            ),
        )
        .unwrap();

        let session = parse_claude_session(&path).unwrap();
        assert_eq!(session.id, id);
        assert_eq!(session.name.as_deref(), Some("Generated title"));
        assert_eq!(session.preview, "last prompt");
        assert_eq!(session.cwd.as_deref(), Some("/tmp/project"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_grok_summary_and_paginates() {
        let id = Uuid::new_v4().to_string();
        let summary: GrokSessionFile = serde_json::from_value(json!({
            "info": { "id": id, "cwd": "/tmp/project" },
            "session_summary": "preview",
            "generated_title": "title",
            "created_at": "2026-07-01T00:00:00Z",
            "updated_at": "2026-07-02T00:00:00Z"
        }))
        .unwrap();
        let session = grok_session_summary(summary, 0).unwrap();
        let page = paginate_sessions(
            vec![session],
            None,
            Some("title".to_string()),
            capabilities(AccountProduct::Grok),
        )
        .unwrap();
        assert_eq!(page.sessions.len(), 1);
        assert!(page.capabilities.can_delete);
        assert!(!page.capabilities.can_rename);
    }

    #[test]
    fn parses_antigravity_workspace_and_timestamp() {
        assert_eq!(
            workspace_path(r#"["file:///tmp/project"]"#).as_deref(),
            Some("/tmp/project")
        );
        assert!(parse_antigravity_timestamp("2026-07-15 03:30:03.720555+00:00").unwrap() > 0);
    }

    #[test]
    fn reads_only_top_level_antigravity_sessions() {
        let root = std::env::temp_dir().join(format!("cortana-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("sessions.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT,
                    title TEXT,
                    preview TEXT,
                    last_modified_time TEXT,
                    workspace_uris TEXT,
                    source TEXT,
                    nesting_depth INTEGER,
                    killed INTEGER
                 );
                 INSERT INTO conversation_summaries VALUES
                    ('top', '', 'Visible', '2026-07-15 03:30:03+00:00',
                     '[\"file:///tmp/project\"]', '', 0, 0),
                    ('nested', '', 'Hidden', '2026-07-15 03:30:03+00:00',
                     '[]', '', 1, 0);",
            )
            .unwrap();
        drop(connection);

        let sessions = list_antigravity_sessions_at(&path).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "top");
        assert_eq!(sessions[0].cwd.as_deref(), Some("/tmp/project"));
        fs::remove_dir_all(root).unwrap();
    }
}
