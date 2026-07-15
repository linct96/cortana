use super::{db::*, *};

pub(super) fn auth_path(state: &AppState) -> Result<PathBuf, String> {
    let connection = open_database(state)?;
    let custom_home = get_setting(&connection, "codex_home")?.unwrap_or_default();
    let home = if custom_home.trim().is_empty() {
        state.default_codex_home.clone()
    } else {
        PathBuf::from(custom_home.trim())
    };
    Ok(home.join("auth.json"))
}

pub(super) fn codex_config_path(state: &AppState) -> Result<PathBuf, String> {
    Ok(auth_path(state)?.with_file_name("config.toml"))
}

#[tauri::command]
pub(super) fn get_codex_config(state: State<'_, AppState>) -> Result<CodexConfigFile, String> {
    read_codex_config(&state)
}

pub(super) fn read_codex_config(state: &AppState) -> Result<CodexConfigFile, String> {
    let path = codex_config_path(state)?;
    let content = if path.exists() {
        fs::read_to_string(&path).map_err(|error| format!("无法读取 Codex config.toml：{error}"))?
    } else {
        String::new()
    };
    Ok(CodexConfigFile {
        path: path.display().to_string(),
        content,
    })
}

#[tauri::command]
pub(super) fn save_codex_config(state: State<'_, AppState>, content: String) -> Result<(), String> {
    save_codex_config_internal(&state, &content)
}

#[tauri::command]
pub(super) fn validate_codex_config(content: String) -> Vec<CodexConfigDiagnostic> {
    let Err(error) = toml::from_str::<toml::Value>(&content) else {
        return Vec::new();
    };
    let span = error.span().unwrap_or(0..0);
    vec![CodexConfigDiagnostic {
        from: byte_offset_to_utf16(&content, span.start),
        to: byte_offset_to_utf16(&content, span.end),
        message: error.message().to_string(),
    }]
}

#[tauri::command]
pub(super) fn format_codex_config(content: String) -> Result<String, String> {
    format_codex_config_internal(&content)
}

pub(super) fn format_codex_config_internal(content: &str) -> Result<String, String> {
    toml::from_str::<toml::Value>(content)
        .map_err(|error| format!("config.toml 格式错误：{error}"))?;
    Ok(taplo::formatter::format(
        content,
        taplo::formatter::Options::default(),
    ))
}

pub(super) fn byte_offset_to_utf16(content: &str, offset: usize) -> usize {
    content
        .char_indices()
        .take_while(|(index, _)| *index < offset)
        .map(|(_, character)| character.len_utf16())
        .sum()
}

pub(super) fn save_codex_config_internal(state: &AppState, content: &str) -> Result<(), String> {
    toml::from_str::<toml::Value>(&content)
        .map_err(|error| format!("config.toml 格式错误：{error}"))?;
    write_file_atomically(&codex_config_path(state)?, content)
}

pub(super) fn update_provider_config_content(
    content: &str,
    account_type: &str,
    api_base_url: Option<&str>,
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
        document["model_provider"] = toml_value(RELAY_MODEL_PROVIDER);
        if document.get("model_providers").is_none() {
            document["model_providers"] = toml_table();
        }
        if document["model_providers"]
            .get(RELAY_MODEL_PROVIDER)
            .is_none()
        {
            document["model_providers"][RELAY_MODEL_PROVIDER] = toml_table();
        }
        document["model_providers"][RELAY_MODEL_PROVIDER]["name"] = toml_value("Relay");
        document["model_providers"][RELAY_MODEL_PROVIDER]["base_url"] = toml_value(api_base_url);
        document["model_providers"][RELAY_MODEL_PROVIDER]["wire_api"] = toml_value("responses");
        document["model_providers"][RELAY_MODEL_PROVIDER]["requires_openai_auth"] =
            toml_value(true);
        document.as_table_mut().remove("openai_base_url");
    } else {
        document.as_table_mut().remove("model_provider");
        document.as_table_mut().remove("openai_base_url");
        document.as_table_mut().remove("model_providers");
    }
    Ok(document.to_string())
}

pub(super) fn read_provider_config(
    path: &Path,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    if !path.exists() {
        return Ok((None, None, None));
    }
    let content =
        fs::read_to_string(path).map_err(|error| format!("无法读取 Codex config.toml：{error}"))?;
    if content.trim().is_empty() {
        return Ok((None, None, None));
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
        .and_then(|item| item.as_str())
        .or_else(|| {
            document
                .get("openai_base_url")
                .and_then(|item| item.as_str())
        });
    Ok((
        model_provider,
        provider_name.map(str::to_string),
        api_base_url.map(str::to_string),
    ))
}

pub(super) fn read_auth_json(path: &Path) -> Result<Option<String>, String> {
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

pub(super) fn write_auth_json_atomically(path: &Path, auth_json: &str) -> Result<(), String> {
    let parsed: Value = serde_json::from_str(auth_json)
        .map_err(|_| "存档的 auth.json 已损坏，拒绝写入。".to_string())?;
    if !parsed.is_object() {
        return Err("存档的 auth.json 格式不正确，拒绝写入。".to_string());
    }
    write_file_atomically(path, auth_json)
}

pub(super) struct ProfileFilesBackup {
    auth_json: Option<String>,
    config: Option<String>,
}

pub(super) fn apply_profile_files(
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
    };
    let next_config = update_provider_config_content(
        backup.config.as_deref().unwrap_or_default(),
        account_type,
        api_base_url,
    )?;
    write_file_atomically(&config_path, &next_config)?;
    if let Err(error) = write_auth_json_atomically(&auth_path, auth_json) {
        restore_optional_file(&config_path, backup.config.as_deref())?;
        return Err(error);
    }
    Ok(backup)
}

pub(super) fn restore_profile_files(
    state: &AppState,
    backup: &ProfileFilesBackup,
) -> Result<(), String> {
    restore_optional_file(&codex_config_path(state)?, backup.config.as_deref())?;
    restore_optional_file(&auth_path(state)?, backup.auth_json.as_deref())
}

pub(super) fn read_optional_file(path: &Path) -> Result<Option<String>, String> {
    if path.exists() {
        fs::read_to_string(path)
            .map(Some)
            .map_err(|error| format!("无法读取 {}：{error}", path.display()))
    } else {
        Ok(None)
    }
}

pub(super) fn restore_optional_file(path: &Path, content: Option<&str>) -> Result<(), String> {
    if let Some(content) = content {
        write_file_atomically(path, content)
    } else if path.exists() {
        fs::remove_file(path).map_err(|error| format!("无法恢复 {}：{error}", path.display()))
    } else {
        Ok(())
    }
}

pub(super) fn write_file_atomically(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Codex 目录无效。".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建 Codex 目录：{error}"))?;
    let temp_path = parent.join(format!(".codex-write-{}.tmp", Uuid::new_v4()));
    {
        let mut temporary = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|error| error.to_string())?;
        temporary
            .write_all(content.as_bytes())
            .map_err(|error| error.to_string())?;
        temporary.sync_all().map_err(|error| error.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    fs::rename(&temp_path, path).map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn normalize_api_base_url(api_base_url: &str) -> Result<String, String> {
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

pub(super) fn build_relay_auth_json(api_key: &str) -> Result<String, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("API Key 不能为空。".to_string());
    }
    serde_json::to_string_pretty(&json!({
        "OPENAI_API_KEY": api_key,
    }))
    .map_err(|error| error.to_string())
}

pub(super) fn extract_api_key(auth_json: &str) -> Result<Option<String>, String> {
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

pub(super) fn extract_api_key_from_value(auth: &Value) -> Option<String> {
    auth.get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|api_key| !api_key.is_empty())
        .map(str::to_string)
}

pub(super) fn has_usable_credential(auth_json: &str) -> bool {
    extract_api_key(auth_json).ok().flatten().is_some() || extract_refresh_token(auth_json).is_ok()
}

pub(super) fn extract_refresh_token(auth_json: &str) -> Result<String, String> {
    if auth_json.len() > MAX_IMPORTED_AUTH_JSON_BYTES {
        return Err("auth.json 内容过大。".to_string());
    }
    let auth: Value =
        serde_json::from_str(auth_json).map_err(|_| "auth.json 不是有效的 JSON。".to_string())?;
    if !auth.is_object() {
        return Err("auth.json 必须是一个 JSON 对象。".to_string());
    }
    auth.get("tokens")
        .and_then(|tokens| tokens.get("refresh_token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "auth.json 缺少 refresh_token。".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
