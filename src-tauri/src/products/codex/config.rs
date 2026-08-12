use crate::{
    features::{
        gateway::{self, UpstreamProtocol},
        models,
    },
    platform::{
        config as platform_config,
        db::{get_setting, open_database},
        files::write_file_atomically,
        state::{
            AppState, ConfigDiagnostic, ConfigFile, ACCOUNT_TYPE_RELAY, CORTANA_MODEL_PROVIDER,
            MAX_IMPORTED_AUTH_JSON_BYTES,
        },
    },
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::State;
use toml_edit::{table as toml_table, value as toml_value, DocumentMut};
use url::Url;

type ProviderConfig = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub(crate) fn auth_path(state: &AppState) -> Result<PathBuf, String> {
    let connection = open_database(state)?;
    let custom_home = get_setting(&connection, "codex_home")?.unwrap_or_default();
    let home = if custom_home.trim().is_empty() {
        state.default_codex_home.clone()
    } else {
        PathBuf::from(custom_home.trim())
    };
    Ok(home.join("auth.json"))
}

pub(crate) fn codex_config_path(state: &AppState) -> Result<PathBuf, String> {
    Ok(auth_path(state)?.with_file_name("config.toml"))
}

pub(crate) fn get_codex_config(state: State<'_, AppState>) -> Result<ConfigFile, String> {
    read_codex_config(&state)
}

pub(crate) fn read_codex_config(state: &AppState) -> Result<ConfigFile, String> {
    platform_config::read_config(&codex_config_path(state)?, "", "Codex config.toml")
}

pub(crate) fn save_codex_config(state: State<'_, AppState>, content: String) -> Result<(), String> {
    save_codex_config_internal(&state, &content)
}

pub(crate) fn validate_codex_config(content: String) -> Vec<ConfigDiagnostic> {
    platform_config::validate_toml(&content)
}

pub(crate) fn format_codex_config(content: String) -> Result<String, String> {
    format_codex_config_internal(&content)
}

pub(crate) fn format_codex_config_internal(content: &str) -> Result<String, String> {
    platform_config::format_toml(content, "config.toml")
}

pub(crate) fn save_codex_config_internal(state: &AppState, content: &str) -> Result<(), String> {
    platform_config::parse_toml(content, "config.toml")?;
    write_file_atomically(&codex_config_path(state)?, content)
}

pub(crate) fn update_provider_config_content(
    content: &str,
    account_type: &str,
    api_base_url: Option<&str>,
    auth_json: &str,
) -> Result<String, String> {
    let mut document = if content.trim().is_empty() {
        DocumentMut::new()
    } else {
        content
            .parse::<DocumentMut>()
            .map_err(|error| format!("config.toml 格式错误：{error}"))?
    };
    if account_type == ACCOUNT_TYPE_RELAY {
        let api_base_url = api_base_url.ok_or_else(|| "中转站账户缺少 API 地址。".to_string())?;
        let api_key =
            extract_api_key(auth_json)?.ok_or_else(|| "中转站账户缺少 API Key。".to_string())?;
        document["forced_login_method"] = toml_value("api");
        document["model_provider"] = toml_value(CORTANA_MODEL_PROVIDER);
        if document.get("model_providers").is_none() {
            document["model_providers"] = toml_table();
        }
        if document["model_providers"]
            .get(CORTANA_MODEL_PROVIDER)
            .is_none()
        {
            document["model_providers"][CORTANA_MODEL_PROVIDER] = toml_table();
        }
        document["model_providers"][CORTANA_MODEL_PROVIDER]["name"] = toml_value("Cortana");
        document["model_providers"][CORTANA_MODEL_PROVIDER]["base_url"] = toml_value(api_base_url);
        document["model_providers"][CORTANA_MODEL_PROVIDER]["experimental_bearer_token"] =
            toml_value(api_key);
        let translated = gateway::is_base_url(api_base_url);
        if let Some(provider) =
            document["model_providers"][CORTANA_MODEL_PROVIDER].as_table_like_mut()
        {
            provider.remove("requires_openai_auth");
            if translated {
                provider.insert("wire_api", toml_value("responses"));
            } else {
                provider.remove("wire_api");
            }
        }
    } else {
        document["forced_login_method"] = toml_value("chatgpt");
        document.as_table_mut().remove("model_provider");
        document.as_table_mut().remove("model_providers");
    }
    Ok(document.to_string())
}

pub(crate) fn read_provider_config(path: &Path) -> Result<ProviderConfig, String> {
    if !path.exists() {
        return Ok((None, None, None, None));
    }
    let content =
        fs::read_to_string(path).map_err(|error| format!("无法读取 Codex config.toml：{error}"))?;
    if content.trim().is_empty() {
        return Ok((None, None, None, None));
    }
    let document = content
        .parse::<DocumentMut>()
        .map_err(|error| format!("config.toml 格式错误：{error}"))?;
    let model_provider = document
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let provider = model_provider.as_deref().and_then(|provider| {
        document
            .get("model_providers")
            .and_then(|item| item.get(provider))
    });
    let provider_name = provider
        .and_then(|item| item.get("name"))
        .and_then(|item| item.as_str());
    let api_base_url = provider
        .and_then(|item| item.get("base_url"))
        .and_then(|item| item.as_str());
    let bearer_token = provider
        .and_then(|item| item.get("experimental_bearer_token"))
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty());
    Ok((
        model_provider,
        provider_name.map(str::to_string),
        api_base_url.map(str::to_string),
        bearer_token.map(str::to_string),
    ))
}

pub(crate) fn read_auth_json(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(path).map_err(|error| format!("无法读取 Codex auth.json：{error}"))?;
    let parsed: Value = serde_json::from_str(&content)
        .map_err(|_| "Codex auth.json 不是有效的 JSON。".to_string())?;
    if !parsed.is_object() {
        return Err("Codex auth.json 必须是一个 JSON 对象。".to_string());
    }
    Ok(Some(content))
}

pub(crate) fn write_auth_json_atomically(path: &Path, auth_json: &str) -> Result<(), String> {
    let parsed: Value = serde_json::from_str(auth_json)
        .map_err(|_| "存档的 auth.json 已损坏，拒绝写入。".to_string())?;
    if !parsed.is_object() {
        return Err("存档的 auth.json 格式不正确，拒绝写入。".to_string());
    }
    write_file_atomically(path, auth_json)
}

pub(crate) struct ProfileFilesBackup {
    auth_json: Option<String>,
    config: Option<String>,
    catalog: Option<(PathBuf, Option<String>)>,
}

pub(crate) fn apply_profile_files(
    state: &AppState,
    auth_json: &str,
    account_type: &str,
    api_base_url: Option<&str>,
) -> Result<ProfileFilesBackup, String> {
    let auth_path = auth_path(state)?;
    let config_path = codex_config_path(state)?;
    let backup = ProfileFilesBackup {
        auth_json: read_optional_file(&auth_path)?,
        config: read_optional_file(&config_path)?,
        catalog: None,
    };
    let next_config = update_provider_config_content(
        backup.config.as_deref().unwrap_or_default(),
        account_type,
        api_base_url,
        auth_json,
    )?;
    write_file_atomically(&config_path, &next_config)?;
    if let Err(error) = apply_auth_file(&auth_path, auth_json, account_type) {
        restore_optional_file(&config_path, backup.config.as_deref())?;
        return Err(error);
    }
    Ok(backup)
}

pub(crate) fn apply_profile_files_with_model(
    state: &AppState,
    auth_json: &str,
    account_type: &str,
    api_base_url: Option<&str>,
    model_profile_id: Option<&str>,
    default_model_id: Option<&str>,
    upstream_protocol: UpstreamProtocol,
) -> Result<ProfileFilesBackup, String> {
    let auth_path = auth_path(state)?;
    let config_path = codex_config_path(state)?;
    let previous_config = read_optional_file(&config_path)?;
    let provider_config = update_provider_config_content(
        previous_config.as_deref().unwrap_or_default(),
        account_type,
        api_base_url,
        auth_json,
    )?;
    let (next_config, catalog) = models::apply_model_config(
        state,
        &provider_config,
        model_profile_id,
        default_model_id,
        upstream_protocol.requires_gateway(),
    )?;
    let previous_catalog = read_optional_file(&catalog.0)?;
    let backup = ProfileFilesBackup {
        auth_json: read_optional_file(&auth_path)?,
        config: previous_config,
        catalog: Some((catalog.0.clone(), previous_catalog)),
    };
    restore_optional_file(&catalog.0, catalog.1.as_deref())?;
    if let Err(error) = write_file_atomically(&config_path, &next_config) {
        restore_catalog(&backup)?;
        return Err(error);
    }
    if let Err(error) = apply_auth_file(&auth_path, auth_json, account_type) {
        restore_optional_file(&config_path, backup.config.as_deref())?;
        restore_catalog(&backup)?;
        return Err(error);
    }
    Ok(backup)
}

pub(crate) fn apply_gateway_profile_files_with_model(
    state: &AppState,
    local_api_key: &str,
    model_profile_id: Option<&str>,
    default_model_id: Option<&str>,
    upstream_protocol: UpstreamProtocol,
) -> Result<ProfileFilesBackup, String> {
    let auth_json = build_relay_auth_json(local_api_key)?;
    apply_profile_files_with_model(
        state,
        &auth_json,
        ACCOUNT_TYPE_RELAY,
        Some(&gateway::base_url()),
        model_profile_id,
        default_model_id,
        upstream_protocol,
    )
}

pub(crate) fn clear_managed_profile_files(state: &AppState) -> Result<ProfileFilesBackup, String> {
    let auth_path = auth_path(state)?;
    let config_path = codex_config_path(state)?;
    let previous_config = read_optional_file(&config_path)?;
    let mut document = previous_config
        .as_deref()
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|error| format!("config.toml 格式错误：{error}"))?;
    document.as_table_mut().remove("forced_login_method");
    if document
        .get("model_provider")
        .and_then(|item| item.as_str())
        == Some(CORTANA_MODEL_PROVIDER)
    {
        document.as_table_mut().remove("model_provider");
    }
    let remove_providers = document
        .get_mut("model_providers")
        .and_then(|item| item.as_table_like_mut())
        .is_some_and(|providers| {
            providers.remove(CORTANA_MODEL_PROVIDER);
            providers.is_empty()
        });
    if remove_providers {
        document.as_table_mut().remove("model_providers");
    }
    let (next_config, catalog) =
        models::apply_model_config(state, &document.to_string(), None, None, false)?;
    let backup = ProfileFilesBackup {
        auth_json: read_optional_file(&auth_path)?,
        config: previous_config,
        catalog: Some((catalog.0.clone(), read_optional_file(&catalog.0)?)),
    };
    restore_optional_file(&catalog.0, None)?;
    if let Err(error) = write_file_atomically(&config_path, &next_config) {
        restore_catalog(&backup)?;
        return Err(error);
    }
    if let Err(error) = restore_optional_file(&auth_path, None) {
        restore_optional_file(&config_path, backup.config.as_deref())?;
        restore_catalog(&backup)?;
        return Err(error);
    }
    Ok(backup)
}

fn apply_auth_file(path: &Path, auth_json: &str, account_type: &str) -> Result<(), String> {
    if account_type != ACCOUNT_TYPE_RELAY {
        return write_auth_json_atomically(path, auth_json);
    }
    if path.exists() {
        fs::remove_file(path).map_err(|error| format!("无法删除旧 Codex auth.json：{error}"))?;
    }
    Ok(())
}

pub(crate) fn restore_profile_files(
    state: &AppState,
    backup: &ProfileFilesBackup,
) -> Result<(), String> {
    restore_optional_file(&codex_config_path(state)?, backup.config.as_deref())?;
    restore_optional_file(&auth_path(state)?, backup.auth_json.as_deref())?;
    restore_catalog(backup)
}

fn restore_catalog(backup: &ProfileFilesBackup) -> Result<(), String> {
    if let Some((path, content)) = backup.catalog.as_ref() {
        restore_optional_file(path, content.as_deref())?;
    }
    Ok(())
}

pub(crate) fn read_optional_file(path: &Path) -> Result<Option<String>, String> {
    if path.exists() {
        fs::read_to_string(path)
            .map(Some)
            .map_err(|error| format!("无法读取 {}：{error}", path.display()))
    } else {
        Ok(None)
    }
}

pub(crate) fn restore_optional_file(path: &Path, content: Option<&str>) -> Result<(), String> {
    if let Some(content) = content {
        write_file_atomically(path, content)
    } else if path.exists() {
        fs::remove_file(path).map_err(|error| format!("无法恢复 {}：{error}", path.display()))
    } else {
        Ok(())
    }
}

pub(crate) fn normalize_api_base_url(api_base_url: &str) -> Result<String, String> {
    let trimmed = api_base_url.trim();
    let url = Url::parse(trimmed).map_err(|_| "API 地址不是有效的 URL。".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("API 地址仅支持 http 或 https。".to_string());
    }
    if url.host_str().is_none() {
        return Err("API 地址缺少主机名。".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("API 地址不能包含用户名或密码。".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("API 地址不能包含查询参数或片段。".to_string());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub(crate) fn build_relay_auth_json(api_key: &str) -> Result<String, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("API Key 不能为空。".to_string());
    }
    serde_json::to_string_pretty(&json!({
        "OPENAI_API_KEY": api_key,
    }))
    .map_err(|error| error.to_string())
}

pub(crate) fn extract_api_key(auth_json: &str) -> Result<Option<String>, String> {
    if auth_json.len() > MAX_IMPORTED_AUTH_JSON_BYTES {
        return Err("auth.json 内容过大。".to_string());
    }
    let auth: Value =
        serde_json::from_str(auth_json).map_err(|_| "auth.json 不是有效的 JSON。".to_string())?;
    if !auth.is_object() {
        return Err("auth.json 必须是一个 JSON 对象。".to_string());
    }
    Ok(extract_api_key_from_value(&auth))
}

pub(crate) fn extract_api_key_from_value(auth: &Value) -> Option<String> {
    auth.get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|api_key| !api_key.is_empty())
        .map(str::to_string)
}

pub(crate) fn has_usable_credential(auth_json: &str) -> bool {
    extract_api_key(auth_json).ok().flatten().is_some() || extract_refresh_token(auth_json).is_ok()
}

pub(crate) fn extract_refresh_token(auth_json: &str) -> Result<String, String> {
    if auth_json.len() > MAX_IMPORTED_AUTH_JSON_BYTES {
        return Err("auth.json 内容过大。".to_string());
    }
    let auth: Value =
        serde_json::from_str(auth_json).map_err(|_| "auth.json 不是有效的 JSON。".to_string())?;
    if !auth.is_object() {
        return Err("auth.json 必须是一个 JSON 对象。".to_string());
    }
    auth.get("refresh_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "auth.json 缺少 refresh_token。".to_string())
}

#[cfg(test)]
mod tests {
    use super::{extract_refresh_token, read_codex_config, save_codex_config_internal, AppState};
    use crate::platform::db::{initialize_database, open_database, set_setting};
    use std::{
        fs,
        sync::{Arc, Mutex},
    };
    use uuid::Uuid;

    #[test]
    fn extracts_top_level_refresh_token() {
        assert_eq!(
            extract_refresh_token(r#"{"refresh_token":" top-level "}"#).unwrap(),
            "top-level"
        );
        assert!(extract_refresh_token(r#"{"tokens":{"refresh_token":"nested"}}"#).is_err());
    }

    #[test]
    fn reads_and_writes_config_from_the_configured_codex_home() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
        let custom_home = directory.join("custom-codex-home");
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        set_setting(
            &open_database(&state).unwrap(),
            "codex_home",
            custom_home.to_str().unwrap(),
        )
        .unwrap();

        let initial = read_codex_config(&state).unwrap();
        assert_eq!(
            initial.path,
            custom_home.join("config.toml").display().to_string()
        );
        assert!(initial.content.is_empty());

        let config = "# keep this comment\nmodel = \"gpt-5.6\"\n[features]\nmemories = true\n";
        save_codex_config_internal(&state, config).unwrap();

        assert_eq!(read_codex_config(&state).unwrap().content, config);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn rejects_invalid_config_without_replacing_the_existing_file() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        save_codex_config_internal(&state, "model = \"gpt-5.6\"\n").unwrap();

        assert!(save_codex_config_internal(&state, "model = [\n").is_err());
        assert_eq!(
            fs::read_to_string(directory.join("config.toml")).unwrap(),
            "model = \"gpt-5.6\"\n"
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
