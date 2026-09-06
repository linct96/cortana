use crate::platform::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

pub(crate) const PI_PROVIDER_OPENAI_CODEX: &str = "openai-codex";
const PI_AGENT_DIR_ENV: &str = "PI_CODING_AGENT_DIR";
pub(crate) type AuthStorage = Map<String, Value>;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenAiCodexCredential {
    pub(crate) r#type: String,
    pub(crate) access: String,
    pub(crate) refresh: String,
    pub(crate) expires: f64,
    pub(crate) account_id: String,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

pub(crate) fn pi_agent_dir(state: &AppState) -> PathBuf {
    let home = state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home);
    resolve_agent_dir(home, std::env::var_os(PI_AGENT_DIR_ENV).as_deref())
}

pub(crate) fn auth_path(state: &AppState) -> PathBuf {
    pi_agent_dir(state).join("auth.json")
}

fn resolve_agent_dir(home: &Path, env_value: Option<&OsStr>) -> PathBuf {
    let Some(value) = env_value.filter(|value| !value.is_empty()) else {
        return home.join(".pi/agent");
    };
    let path = PathBuf::from(value);
    if path == Path::new("~") {
        home.to_path_buf()
    } else if let Ok(rest) = path.strip_prefix("~") {
        home.join(rest)
    } else {
        path
    }
}

pub(crate) fn parse_openai_codex_credential(
    value: &Value,
) -> Result<OpenAiCodexCredential, String> {
    let credential: OpenAiCodexCredential =
        serde_json::from_value(value.clone()).map_err(|_| unsupported_credential_error())?;
    if credential.r#type != "oauth"
        || credential.access.trim().is_empty()
        || credential.refresh.trim().is_empty()
        || !credential.expires.is_finite()
        || credential.account_id.trim().is_empty()
    {
        return Err(unsupported_credential_error());
    }
    Ok(credential)
}

pub(crate) fn read_auth_storage(path: &Path) -> Result<AuthStorage, String> {
    read_json_storage(path, "auth.json")
}

pub(crate) fn read_json_storage(path: &Path, name: &str) -> Result<AuthStorage, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(format!("无法读取 Pi {name}：{error}")),
    };
    let value: Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
            .map_err(|_| format!("Pi {name} 不是有效的 JSON，已停止操作以避免覆盖现有配置。"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("Pi {name} 不是有效的 JSON 对象，已停止操作。"))
}

pub(crate) fn openai_codex_credential(
    storage: &AuthStorage,
) -> Result<Option<OpenAiCodexCredential>, String> {
    storage
        .get(PI_PROVIDER_OPENAI_CODEX)
        .map(parse_openai_codex_credential)
        .transpose()
}

pub(crate) fn merge_openai_codex(
    mut storage: AuthStorage,
    credential: &OpenAiCodexCredential,
) -> Result<AuthStorage, String> {
    storage.insert(
        PI_PROVIDER_OPENAI_CODEX.to_string(),
        serde_json::to_value(credential).map_err(|error| error.to_string())?,
    );
    Ok(storage)
}

pub(crate) fn credential_json(credential: &OpenAiCodexCredential) -> Result<String, String> {
    serde_json::to_string(credential).map_err(|error| error.to_string())
}

pub(crate) fn write_auth_storage(path: &Path, storage: &AuthStorage) -> Result<(), String> {
    write_json_storage(path, storage)
}

pub(crate) fn write_json_storage(path: &Path, storage: &AuthStorage) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(storage).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    write_auth_bytes(path, &bytes)
}

fn write_auth_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Pi 认证路径无效。".to_string())?;
    create_private_dir(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let temporary = parent.join(format!(".auth.json.{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mode = fs::metadata(path)
                .map(|metadata| metadata.permissions().mode() & 0o777)
                .unwrap_or(0o600);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(mode)
                .open(&temporary)
                .map_err(|error| format!("无法写入 Pi auth.json：{error}"))?;
            std::io::Write::write_all(&mut file, bytes).map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            fs::rename(&temporary, path).map_err(|error| error.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    #[cfg(not(unix))]
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|error| format!("无法写入 Pi auth.json：{error}"))?;
        std::io::Write::write_all(&mut file, bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())
    }
}

pub(crate) fn restore_auth_contents(path: &Path, original: Option<&[u8]>) -> Result<(), String> {
    match original {
        Some(bytes) => write_auth_bytes(path, bytes),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        },
    }
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    let existed = path.exists();
    fs::create_dir_all(path).map_err(|error| format!("无法创建 Pi 配置目录：{error}"))?;
    #[cfg(unix)]
    if !existed {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn unsupported_credential_error() -> String {
    "当前 Pi openai-codex 凭据格式不受此版本 Cortana 支持。".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_merges_only_openai_codex() {
        let mut storage = serde_json::json!({
            "anthropic": {"type": "oauth", "nested": {"keep": true}}
        })
        .as_object()
        .unwrap()
        .clone();
        let credential = parse_openai_codex_credential(&serde_json::json!({
            "type": "oauth", "access": "a", "refresh": "r", "expires": 1, "accountId": "acct"
        }))
        .unwrap();
        let anthropic = storage["anthropic"].clone();
        storage = merge_openai_codex(storage, &credential).unwrap();
        assert_eq!(storage["anthropic"], anthropic);
        assert_eq!(
            openai_codex_credential(&storage).unwrap().unwrap(),
            credential
        );
    }

    #[test]
    fn resolves_default_and_tilde_override() {
        let home = Path::new("/home/test");
        assert_eq!(resolve_agent_dir(home, None), home.join(".pi/agent"));
        assert_eq!(
            resolve_agent_dir(home, Some(OsStr::new("~/pi"))),
            home.join("pi")
        );
    }
}
