use super::{codex::auth_path, *};
#[cfg(target_os = "macos")]
use std::os::unix::fs::OpenOptionsExt;

const CODEX_RELEASE_URL: &str = "https://api.github.com/repos/openai/codex/releases/latest";
const CLAUDE_RELEASE_URL: &str = "https://downloads.claude.ai/claude-code-releases/latest";
const ANTIGRAVITY_RELEASE_URL: &str =
    "https://api.github.com/repos/google-antigravity/antigravity-cli/releases/latest";
const GROK_RELEASE_URL: &str = "https://x.ai/cli/stable";
const TERMINAL_APP_SETTING: &str = "terminal_app";
const DEFAULT_TERMINAL_APP: &str = "terminal";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CliEnvironment {
    installed: bool,
    installed_version: Option<String>,
    latest_version: Option<String>,
    install_method: String,
}

#[tauri::command]
pub(super) async fn is_codex_cli_available(state: State<'_, AppState>) -> Result<bool, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || Ok(find_codex(&state).is_some()))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn is_claude_cli_available(state: State<'_, AppState>) -> Result<bool, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || Ok(find_claude(&state).is_some()))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn is_antigravity_cli_available(
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || Ok(find_antigravity(&state).is_some()))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn is_grok_cli_available(state: State<'_, AppState>) -> Result<bool, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || Ok(find_grok(&state).is_some()))
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
pub(super) async fn get_antigravity_cli_environment(
    state: State<'_, AppState>,
) -> Result<CliEnvironment, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || inspect_antigravity_environment(&state))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn get_grok_cli_environment(
    state: State<'_, AppState>,
) -> Result<CliEnvironment, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || inspect_grok_environment(&state))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) fn get_terminal_app(state: State<'_, AppState>) -> Result<String, String> {
    let connection = db::open_database(&state)?;
    Ok(normalize_terminal_app(db::get_setting(
        &connection,
        TERMINAL_APP_SETTING,
    )?))
}

#[tauri::command]
pub(super) fn set_terminal_app(
    state: State<'_, AppState>,
    terminal_app: String,
) -> Result<String, String> {
    terminal_application(&terminal_app)?;
    let connection = db::open_database(&state)?;
    db::set_setting(&connection, TERMINAL_APP_SETTING, &terminal_app)?;
    Ok(terminal_app)
}

pub(super) fn codex_candidates(state: &AppState) -> Vec<PathBuf> {
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

pub(super) fn find_codex(state: &AppState) -> Option<(PathBuf, String)> {
    codex_candidates(state).into_iter().find_map(|candidate| {
        let output = codex_command(&candidate, &["--version"]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let version = parse_codex_version(&String::from_utf8_lossy(&output.stdout))?;
        Some((candidate, version))
    })
}

#[cfg(target_os = "macos")]
pub(super) fn open_codex_cli(
    state: &AppState,
    environment: &[(String, String)],
    arguments: &[String],
) -> Result<(), String> {
    let (candidate, _) = find_codex(state)
        .ok_or_else(|| "未找到可用的 Codex CLI，请先安装或更新 Codex。".to_string())?;
    let home = state
        .default_codex_home
        .parent()
        .ok_or_else(|| "无法定位用户主目录。".to_string())?;
    let launcher = home.join(format!(".cortana-codex-{}.command", Uuid::new_v4()));
    let script = codex_launcher_script(home, &candidate, environment, arguments);
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&launcher)
            .map_err(|error| format!("无法创建 CLI 启动脚本：{error}"))?;
        file.write_all(script.as_bytes())
            .map_err(|error| format!("无法写入 CLI 启动脚本：{error}"))?;
        file.sync_all()
            .map_err(|error| format!("无法保存 CLI 启动脚本：{error}"))
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&launcher);
        return Err(error);
    }

    let terminal_app = {
        let connection = db::open_database(state)?;
        normalize_terminal_app(db::get_setting(&connection, TERMINAL_APP_SETTING)?)
    };
    let application = terminal_application(&terminal_app)?;
    let status = Command::new("/usr/bin/open")
        .args(["-a", application])
        .arg(&launcher)
        .status();
    if !status.is_ok_and(|status| status.success()) {
        let _ = fs::remove_file(&launcher);
        return Err(format!(
            "无法使用 {application} 打开终端，请确认应用已安装。"
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn open_codex_cli(
    _state: &AppState,
    _environment: &[(String, String)],
    _arguments: &[String],
) -> Result<(), String> {
    Err("使用指定账号打开 Codex CLI 目前仅支持 macOS。".to_string())
}

#[cfg(any(target_os = "macos", test))]
fn codex_launcher_script(
    home: &Path,
    candidate: &Path,
    environment: &[(String, String)],
    arguments: &[String],
) -> String {
    let exports = environment
        .iter()
        .map(|(name, value)| format!("export {name}={}\n", shell_quote(value)))
        .collect::<String>();
    let arguments = arguments
        .iter()
        .map(|argument| format!(" {}", shell_quote(argument)))
        .collect::<String>();
    format!(
        "#!/bin/zsh\nrm -f -- \"$0\"\ncd -- {} || exit 1\n{exports}exec {}{arguments}\n",
        shell_quote(&home.display().to_string()),
        shell_quote(&candidate.display().to_string()),
    )
}

#[cfg(any(target_os = "macos", test))]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn terminal_application(value: &str) -> Result<&'static str, String> {
    match value {
        "terminal" => Ok("Terminal"),
        "warp" => Ok("Warp"),
        "ghostty" => Ok("Ghostty"),
        _ => Err("不支持的终端应用。".to_string()),
    }
}

fn normalize_terminal_app(value: Option<String>) -> String {
    value
        .filter(|value| terminal_application(value).is_ok())
        .unwrap_or_else(|| DEFAULT_TERMINAL_APP.to_string())
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
    let found = find_claude(state);

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

fn find_claude(state: &AppState) -> Option<(PathBuf, String)> {
    let home = state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home);
    claude_candidates(home).into_iter().find_map(|candidate| {
        let output = codex_command(&candidate, &["--version"]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let version = parse_claude_version(&String::from_utf8_lossy(&output.stdout))?;
        Some((candidate, version))
    })
}

fn inspect_antigravity_environment(state: &AppState) -> Result<CliEnvironment, String> {
    let found = find_antigravity(state);

    Ok(CliEnvironment {
        installed: found.is_some(),
        installed_version: found.as_ref().map(|(_, version)| version.clone()),
        latest_version: fetch_latest_antigravity_version(),
        install_method: found
            .as_ref()
            .map(|(candidate, _)| antigravity_install_method(candidate, antigravity_home(state)))
            .unwrap_or("unknown")
            .to_string(),
    })
}

fn inspect_grok_environment(state: &AppState) -> Result<CliEnvironment, String> {
    let found = find_grok(state);
    Ok(CliEnvironment {
        installed: found.is_some(),
        installed_version: found.as_ref().map(|(_, version)| version.clone()),
        latest_version: fetch_latest_grok_version(),
        install_method: found
            .as_ref()
            .map(|(candidate, _)| grok_install_method(candidate, grok_home(state)))
            .unwrap_or("unknown")
            .to_string(),
    })
}

pub(super) fn grok_home(state: &AppState) -> PathBuf {
    std::env::var_os("GROK_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| antigravity_home(state).join(".grok"))
}

fn find_grok(state: &AppState) -> Option<(PathBuf, String)> {
    grok_candidates(state).into_iter().find_map(|candidate| {
        let output = codex_command(&candidate, &["--version"]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        parse_grok_version(&stdout)
            .or_else(|| parse_grok_version(&stderr))
            .map(|version| (candidate, version))
    })
}

pub(super) fn grok_candidates(state: &AppState) -> Vec<PathBuf> {
    let home = grok_home(state);
    #[cfg(not(target_os = "windows"))]
    let mut candidates = vec![home.join("bin/grok")];
    #[cfg(target_os = "windows")]
    let mut candidates = vec![home.join("bin/grok.exe")];
    #[cfg(target_os = "macos")]
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/grok"),
        PathBuf::from("/usr/local/bin/grok"),
    ]);
    #[cfg(not(target_os = "windows"))]
    candidates.push(PathBuf::from("grok"));
    #[cfg(target_os = "windows")]
    candidates.extend([PathBuf::from("grok.exe"), PathBuf::from("grok")]);
    candidates
}

fn grok_install_method(candidate: &Path, home: PathBuf) -> &'static str {
    if candidate.starts_with(home) {
        "standalone"
    } else if candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf())
        .to_string_lossy()
        .to_ascii_lowercase()
        .contains("homebrew")
    {
        "brew"
    } else {
        "unknown"
    }
}

fn antigravity_home(state: &AppState) -> &Path {
    state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home)
}

fn find_antigravity(state: &AppState) -> Option<(PathBuf, String)> {
    antigravity_candidates(antigravity_home(state))
        .into_iter()
        .find_map(|candidate| {
            let output = codex_command(&candidate, &["--version"]).output().ok()?;
            if !output.status.success() {
                return None;
            }
            let version = parse_cli_version(&String::from_utf8_lossy(&output.stdout))?;
            Some((candidate, version))
        })
}

fn antigravity_candidates(home: &Path) -> Vec<PathBuf> {
    #[cfg(not(target_os = "windows"))]
    let mut candidates = vec![home.join(".local/bin/agy")];
    #[cfg(target_os = "windows")]
    let mut candidates = vec![home.join("AppData/Local/agy/bin/agy.exe")];
    #[cfg(target_os = "macos")]
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/agy"),
        PathBuf::from("/usr/local/bin/agy"),
    ]);
    #[cfg(not(target_os = "windows"))]
    candidates.push(PathBuf::from("agy"));
    #[cfg(target_os = "windows")]
    candidates.extend([PathBuf::from("agy.exe"), PathBuf::from("agy")]);
    candidates
}

fn antigravity_install_method(candidate: &Path, home: &Path) -> &'static str {
    if candidate.starts_with(home.join(".local"))
        || candidate.starts_with(home.join("AppData/Local/agy"))
    {
        return "standalone";
    }
    let path = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf())
        .to_string_lossy()
        .to_ascii_lowercase();
    if path.contains("homebrew") || path.contains("cellar/antigravity-cli") {
        "brew"
    } else {
        "unknown"
    }
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
    parse_cli_version(value)
}

fn parse_cli_version(value: &str) -> Option<String> {
    let version = value.split_whitespace().next()?.trim_start_matches('v');
    (!version.is_empty()
        && version
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-+".contains(character)))
    .then(|| version.to_string())
}

fn parse_grok_version(value: &str) -> Option<String> {
    value
        .split_whitespace()
        .find(|part| part.contains('.') && part.chars().any(|character| character.is_ascii_digit()))
        .and_then(parse_cli_version)
}

fn fetch_latest_antigravity_version() -> Option<String> {
    let release: Value = Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("Cortana/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?
        .get(ANTIGRAVITY_RELEASE_URL)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .ok()?
        .json()
        .ok()?;
    release
        .get("tag_name")
        .and_then(Value::as_str)
        .and_then(parse_cli_version)
}

fn fetch_latest_grok_version() -> Option<String> {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("Cortana/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?
        .get(GROK_RELEASE_URL)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .ok()?
        .text()
        .ok()
        .and_then(|version| parse_grok_version(&version))
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

pub(super) fn codex_command(candidate: &Path, arguments: &[&str]) -> Command {
    #[allow(unused_mut)]
    let mut command = if candidate
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
    };

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    command
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn builds_self_deleting_codex_launcher_with_shell_escaping() {
        let script = codex_launcher_script(
            Path::new("/Users/O'Brien"),
            Path::new("/opt/homebrew/bin/codex"),
            &[("CODEX_ACCESS_TOKEN".to_string(), "token'quoted".to_string())],
            &["-c".to_string(), "model_provider=\"openai\"".to_string()],
        );
        assert!(script.contains("rm -f -- \"$0\""));
        assert!(script.contains("cd -- '/Users/O'\\''Brien'"));
        assert!(script.contains("export CODEX_ACCESS_TOKEN='token'\\''quoted'"));
        assert!(script.contains("exec '/opt/homebrew/bin/codex' '-c'"));
        assert!(script.ends_with("'model_provider=\"openai\"'\n"));
    }

    #[test]
    fn validates_and_defaults_terminal_app() {
        assert_eq!(terminal_application("terminal"), Ok("Terminal"));
        assert_eq!(terminal_application("warp"), Ok("Warp"));
        assert_eq!(terminal_application("ghostty"), Ok("Ghostty"));
        assert_eq!(normalize_terminal_app(None), "terminal");
        assert_eq!(
            normalize_terminal_app(Some("unsupported".to_string())),
            "terminal"
        );
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

    #[test]
    fn parses_antigravity_environment_metadata() {
        assert_eq!(parse_cli_version("1.1.3\n").as_deref(), Some("1.1.3"));
        assert_eq!(parse_cli_version("v1.1.3").as_deref(), Some("1.1.3"));
        assert_eq!(
            antigravity_install_method(
                Path::new("/Users/test/.local/bin/agy"),
                Path::new("/Users/test")
            ),
            "standalone"
        );
    }

    #[test]
    fn parses_grok_environment_metadata() {
        assert_eq!(
            parse_grok_version("grok 0.2.106").as_deref(),
            Some("0.2.106")
        );
        assert_eq!(
            parse_grok_version("grok 0.2.106 (bde89716f679)").as_deref(),
            Some("0.2.106")
        );
        assert_eq!(parse_grok_version("0.2.106\n").as_deref(), Some("0.2.106"));
    }
}
