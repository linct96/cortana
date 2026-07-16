use super::{codex::auth_path, *};

const CODEX_RELEASE_URL: &str = "https://api.github.com/repos/openai/codex/releases/latest";
const CLAUDE_RELEASE_URL: &str = "https://downloads.claude.ai/claude-code-releases/latest";

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

pub(super) fn codex_command(candidate: &Path, arguments: &[&str]) -> Command {
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
