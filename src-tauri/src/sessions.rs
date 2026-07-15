use super::{codex::auth_path, *};
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
};

const REQUEST_ID: u64 = 1;
const PAGE_SIZE: u32 = 50;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CODEX_RELEASE_URL: &str = "https://api.github.com/repos/openai/codex/releases/latest";
const CLAUDE_RELEASE_URL: &str = "https://downloads.claude.ai/claude-code-releases/latest";

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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CliEnvironment {
    installed: bool,
    installed_version: Option<String>,
    latest_version: Option<String>,
    install_method: String,
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
pub(super) async fn is_codex_cli_available(state: State<'_, AppState>) -> Result<bool, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || Ok(find_codex(&state).is_some()))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn get_codex_cli_environment(
    state: State<'_, AppState>,
) -> Result<CliEnvironment, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || inspect_codex_environment(&state))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn get_claude_cli_environment(
    state: State<'_, AppState>,
) -> Result<CliEnvironment, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || inspect_claude_environment(&state))
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

fn codex_candidates(state: &AppState) -> Vec<PathBuf> {
    let home = state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home);
    #[cfg(not(target_os = "windows"))]
    let mut candidates = vec![PathBuf::from("codex")];
    #[cfg(target_os = "windows")]
    let mut candidates = vec![PathBuf::from("codex.cmd")];
    #[cfg(target_os = "macos")]
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
        home.join(".local/bin/codex"),
    ]);
    #[cfg(target_os = "windows")]
    candidates.extend([
        home.join("AppData/Roaming/npm/codex.cmd"),
        PathBuf::from("codex"),
        PathBuf::from("codex.exe"),
    ]);

    candidates
}

fn find_codex(state: &AppState) -> Option<(PathBuf, String)> {
    codex_candidates(state).into_iter().find_map(|candidate| {
        let output = codex_command(&candidate, &["--version"]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let version = parse_codex_version(&String::from_utf8_lossy(&output.stdout))?;
        Some((candidate, version))
    })
}

fn inspect_codex_environment(state: &AppState) -> Result<CliEnvironment, String> {
    let Some((candidate, fallback_version)) = find_codex(state) else {
        return Ok(CliEnvironment {
            installed: false,
            installed_version: None,
            latest_version: fetch_latest_codex_version(),
            install_method: "unknown".to_string(),
        });
    };

    let codex_home = auth_path(state)
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let mut command = codex_command(&candidate, &["doctor", "--json"]);
    if let Some(codex_home) = codex_home {
        command.env("CODEX_HOME", codex_home);
    }
    let report = command
        .output()
        .ok()
        .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok());
    let (doctor_version, doctor_latest, install_method) = report
        .as_ref()
        .map(parse_doctor_environment)
        .unwrap_or((None, None, "unknown"));

    Ok(CliEnvironment {
        installed: true,
        installed_version: Some(doctor_version.unwrap_or(fallback_version)),
        latest_version: doctor_latest.or_else(fetch_latest_codex_version),
        install_method: install_method.to_string(),
    })
}

fn inspect_claude_environment(state: &AppState) -> Result<CliEnvironment, String> {
    let home = state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home);
    let found = claude_candidates(home).into_iter().find_map(|candidate| {
        let output = codex_command(&candidate, &["--version"]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let version = parse_claude_version(&String::from_utf8_lossy(&output.stdout))?;
        Some((candidate, version))
    });

    Ok(CliEnvironment {
        installed: found.is_some(),
        installed_version: found.as_ref().map(|(_, version)| version.clone()),
        latest_version: fetch_latest_claude_version(),
        install_method: found
            .as_ref()
            .map(|(candidate, _)| claude_install_method(candidate, home))
            .unwrap_or("unknown")
            .to_string(),
    })
}

fn claude_candidates(home: &Path) -> Vec<PathBuf> {
    #[cfg(not(target_os = "windows"))]
    let mut candidates = vec![home.join(".local/bin/claude")];
    #[cfg(target_os = "windows")]
    let mut candidates = vec![home.join(".local/bin/claude.exe")];
    #[cfg(target_os = "macos")]
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
    ]);
    #[cfg(not(target_os = "windows"))]
    candidates.extend([home.join(".claude/local/claude"), PathBuf::from("claude")]);
    #[cfg(target_os = "windows")]
    candidates.extend([
        home.join("AppData/Roaming/npm/claude.cmd"),
        PathBuf::from("claude.cmd"),
        PathBuf::from("claude.exe"),
    ]);
    candidates
}

fn claude_install_method(candidate: &Path, home: &Path) -> &'static str {
    if candidate.starts_with(home.join(".local")) {
        return "standalone";
    }
    let path = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf())
        .to_string_lossy()
        .to_ascii_lowercase();
    if path.contains("caskroom/claude-code") {
        "brew"
    } else if path.contains("node_modules")
        || path.contains(".claude/local")
        || path.contains("appdata/roaming/npm")
    {
        "npm"
    } else {
        "unknown"
    }
}

fn fetch_latest_claude_version() -> Option<String> {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("Cortana/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?
        .get(CLAUDE_RELEASE_URL)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .ok()?
        .text()
        .ok()
        .and_then(|version| parse_claude_version(&version))
}

fn parse_claude_version(value: &str) -> Option<String> {
    let version = value.split_whitespace().next()?.trim_start_matches('v');
    (!version.is_empty()
        && version
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-+".contains(character)))
    .then(|| version.to_string())
}

fn parse_doctor_environment(report: &Value) -> (Option<String>, Option<String>, &'static str) {
    let installed_version = report
        .get("codexVersion")
        .and_then(Value::as_str)
        .and_then(parse_codex_version);
    let latest_version = report
        .pointer("/checks/updates.status/details/latest version")
        .and_then(Value::as_str)
        .and_then(parse_codex_version);
    let install_method = report
        .pointer("/checks/runtime.provenance/details/install method")
        .or_else(|| report.pointer("/checks/installation/details/install context"))
        .and_then(Value::as_str)
        .map(normalize_install_method)
        .unwrap_or("unknown");
    (installed_version, latest_version, install_method)
}

fn fetch_latest_codex_version() -> Option<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("Cortana/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let release: Value = client
        .get(CODEX_RELEASE_URL)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .ok()?
        .json()
        .ok()?;
    release
        .get("tag_name")
        .and_then(Value::as_str)
        .and_then(parse_codex_version)
}

fn parse_codex_version(value: &str) -> Option<String> {
    let value = value.split_whitespace().last()?.trim();
    let value = value
        .strip_prefix("rust-v")
        .or_else(|| value.strip_prefix('v'))
        .unwrap_or(value);
    (!value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-+".contains(character)))
    .then(|| value.to_string())
}

fn normalize_install_method(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "brew" | "homebrew" => "brew",
        "npm" => "npm",
        "pnpm" => "pnpm",
        "bun" => "bun",
        "standalone" | "installer" => "standalone",
        _ => "unknown",
    }
}

fn codex_command(candidate: &Path, arguments: &[&str]) -> Command {
    if candidate
        .extension()
        .is_some_and(|extension| extension == "cmd")
    {
        let mut command = Command::new("cmd");
        command.args(["/D", "/C"]).arg(candidate).args(arguments);
        command
    } else {
        let mut command = Command::new(candidate);
        command.args(arguments);
        command
    }
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
    let _ = Command::new("taskkill")
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
    fn parses_codex_environment_metadata() {
        assert_eq!(
            parse_codex_version("codex-cli 0.144.4").as_deref(),
            Some("0.144.4")
        );
        assert_eq!(
            parse_codex_version("rust-v0.144.4").as_deref(),
            Some("0.144.4")
        );
        assert_eq!(normalize_install_method("Homebrew"), "brew");
        assert_eq!(normalize_install_method("unexpected"), "unknown");

        let report = json!({
            "codexVersion": "0.143.0",
            "checks": {
                "runtime.provenance": { "details": { "install method": "brew" } },
                "updates.status": { "details": { "latest version": "0.144.4" } }
            }
        });
        let (installed, latest, method) = parse_doctor_environment(&report);
        assert_eq!(installed.as_deref(), Some("0.143.0"));
        assert_eq!(latest.as_deref(), Some("0.144.4"));
        assert_eq!(method, "brew");
    }

    #[test]
    fn parses_claude_environment_metadata() {
        assert_eq!(
            parse_claude_version("2.1.199 (Claude Code)").as_deref(),
            Some("2.1.199")
        );
        assert_eq!(
            parse_claude_version("v2.1.210\n").as_deref(),
            Some("2.1.210")
        );
        assert_eq!(
            claude_install_method(
                Path::new("/Users/test/.local/bin/claude"),
                Path::new("/Users/test")
            ),
            "standalone"
        );
    }
}
