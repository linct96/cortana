use super::{profiles::*, *};

const CODEX_CLI_TOKEN_REFRESH_BUFFER_SECONDS: i64 = 60 * 60;
pub(super) const CODEX_RELAY_API_KEY_ENV: &str = "CORTANA_CODEX_RELAY_API_KEY";
// ponytail: 账号量很小；出现可测量的跨账号等待时再拆为每账号锁。
static CODEX_AUTH_REFRESH_LOCK: Mutex<()> = Mutex::new(());

pub(super) struct CodexApiError {
    pub(super) message: String,
    pub(super) unauthorized: bool,
    pub(super) authentication_invalidated: bool,
}

pub(super) fn backend_error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .or_else(|| value.get("error").and_then(|error| error.get("message")))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string())
}

pub(super) fn authentication_invalidated(status: u16, body: &str) -> bool {
    if status != 401 {
        return false;
    }
    let body = body.to_ascii_lowercase();
    [
        "token_invalidated",
        "invalid_token",
        "authentication token has been invalidated",
    ]
    .iter()
    .any(|signal| body.contains(signal))
}

pub(super) fn open_codex_cli_with_profile_internal(
    state: &AppState,
    profile_id: &str,
) -> Result<(), String> {
    let connection = open_database(state)?;
    let (account_type, api_base_url, auth_json) = connection
        .query_row(
            "SELECT account_type, api_base_url, auth_json FROM accounts WHERE id = ?1 AND product = 'codex'",
            params![profile_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "Codex 账户不存在。".to_string())?;

    let (environment, arguments) = if account_type == ACCOUNT_TYPE_OAUTH {
        let auth = ensure_fresh_codex_auth(state, profile_id, None)?;
        (
            vec![(
                "CODEX_ACCESS_TOKEN".to_string(),
                codex_access_token(&auth.auth_json)?,
            )],
            vec![
                "-c".to_string(),
                codex_cli_config_override("model_provider", "openai"),
            ],
        )
    } else if account_type == ACCOUNT_TYPE_RELAY {
        let api_base_url = api_base_url.ok_or_else(|| "中转站账户缺少 API 地址。".to_string())?;
        let api_key =
            extract_api_key(&auth_json)?.ok_or_else(|| "中转站账户缺少 API Key。".to_string())?;
        codex_relay_cli_options(api_key, &api_base_url)
    } else {
        return Err("不支持该 Codex 账户类型。".to_string());
    };

    env::open_codex_cli(state, &environment, &arguments)
}

pub(super) fn codex_access_token(auth_json: &str) -> Result<String, String> {
    serde_json::from_str::<Value>(auth_json)
        .map_err(|_| "存档的 auth.json 已损坏。".to_string())?
        .get("tokens")
        .and_then(|tokens| tokens.get("access_token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "账户缺少 access_token，请重新授权。".to_string())
}

pub(super) fn codex_token_needs_refresh(auth_json: &str, now: i64) -> Result<bool, String> {
    let access_token = match codex_access_token(auth_json) {
        Ok(token) => token,
        Err(_) => return Ok(true),
    };
    Ok(decode_jwt_claims(&access_token)
        .and_then(|claims| claims.get("exp").and_then(Value::as_i64))
        .is_some_and(|expires_at| expires_at <= now + CODEX_CLI_TOKEN_REFRESH_BUFFER_SECONDS))
}

pub(super) fn codex_auth_needs_refresh(
    auth_json: &str,
    rejected_auth: Option<&str>,
    now: i64,
) -> Result<bool, String> {
    rejected_auth.map_or_else(
        || codex_token_needs_refresh(auth_json, now),
        |rejected| Ok(auth_json == rejected),
    )
}

#[derive(Clone)]
pub(super) struct CodexAuth {
    pub(super) account_id: String,
    pub(super) auth_json: String,
}

pub(super) fn ensure_fresh_codex_auth(
    state: &AppState,
    profile_id: &str,
    rejected_auth: Option<&str>,
) -> Result<CodexAuth, String> {
    let _guard = CODEX_AUTH_REFRESH_LOCK
        .lock()
        .map_err(|_| "Codex 认证刷新锁不可用。".to_string())?;
    let connection = open_database(state)?;
    let (_, active_id) = resolve_auth_state(&connection, &auth_path(state)?)?;
    let (account_type, account_id, mut auth_json) = connection
        .query_row(
            "SELECT account_type, account_id, auth_json
             FROM accounts WHERE id = ?1 AND product = 'codex'",
            params![profile_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type != ACCOUNT_TYPE_OAUTH {
        return Err("中转站账户不支持 OAuth 认证刷新。".to_string());
    }
    let needs_refresh =
        codex_auth_needs_refresh(&auth_json, rejected_auth, Utc::now().timestamp())?;
    if needs_refresh {
        let refresh_token = match extract_refresh_token(&auth_json) {
            Ok(refresh_token) => refresh_token,
            Err(error) => {
                mark_codex_oauth_invalidated(&connection, profile_id)?;
                return Err(error);
            }
        };
        let token = match refresh_oauth_token_detailed(&refresh_token) {
            Ok(token) => token,
            Err(error) => {
                if error.reauthorization_required {
                    mark_codex_oauth_invalidated(&connection, profile_id)?;
                }
                return Err(error.message);
            }
        };
        auth_json = build_codex_auth_json(&token)?;
        persist_codex_cli_auth(
            state,
            &connection,
            profile_id,
            &auth_json,
            active_id.as_deref() == Some(profile_id),
        )?;
    }
    Ok(CodexAuth {
        account_id,
        auth_json,
    })
}

pub(super) fn mark_codex_oauth_invalidated(
    connection: &Connection,
    profile_id: &str,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE accounts SET oauth_invalidated_at = ?1
             WHERE id = ?2 AND product = 'codex'",
            params![now_millis(), profile_id],
        )
        .map_err(database_error)?;
    Ok(())
}

pub(super) fn with_codex_auth_retry<T>(
    state: &AppState,
    profile_id: &str,
    request: impl Fn(&CodexAuth) -> Result<T, CodexApiError>,
) -> Result<T, String> {
    let auth = ensure_fresh_codex_auth(state, profile_id, None)?;
    match request(&auth) {
        Ok(value) => Ok(value),
        Err(error) if error.unauthorized => {
            let refreshed =
                ensure_fresh_codex_auth(state, profile_id, Some(auth.auth_json.as_str()))?;
            finish_codex_request(state, profile_id, request(&refreshed))
        }
        Err(error) => finish_codex_request(state, profile_id, Err(error)),
    }
}

pub(super) fn finish_codex_request<T>(
    state: &AppState,
    profile_id: &str,
    result: Result<T, CodexApiError>,
) -> Result<T, String> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if error.authentication_invalidated {
                let connection = open_database(state)?;
                mark_codex_oauth_invalidated(&connection, profile_id)?;
            }
            Err(error.message)
        }
    }
}

pub(super) fn persist_codex_cli_auth(
    state: &AppState,
    connection: &Connection,
    profile_id: &str,
    auth_json: &str,
    active: bool,
) -> Result<(), String> {
    let path = active.then(|| auth_path(state)).transpose()?;
    let backup = path
        .as_ref()
        .map(|path| read_optional_file(path))
        .transpose()?
        .flatten();
    if let Some(path) = path.as_ref() {
        write_auth_json_atomically(path, auth_json)?;
    }
    if let Err(error) = connection
        .execute(
            "UPDATE accounts SET auth_json = ?1, oauth_invalidated_at = NULL, updated_at = ?2 WHERE id = ?3 AND product = 'codex'",
            params![auth_json, now_millis(), profile_id],
        )
        .map_err(database_error)
    {
        if let Some(path) = path.as_ref() {
            restore_optional_file(path, backup.as_deref())?;
        }
        return Err(error);
    }
    Ok(())
}

pub(super) fn codex_cli_config_override(key: &str, value: &str) -> String {
    format!("{key}={}", toml_edit::Value::from(value))
}

pub(super) fn codex_relay_cli_options(
    api_key: String,
    api_base_url: &str,
) -> (Vec<(String, String)>, Vec<String>) {
    (
        vec![(CODEX_RELAY_API_KEY_ENV.to_string(), api_key)],
        vec![
            "-c".to_string(),
            codex_cli_config_override("model_provider", "cortana_relay"),
            "-c".to_string(),
            codex_cli_config_override("model_providers.cortana_relay.name", "Relay"),
            "-c".to_string(),
            codex_cli_config_override("model_providers.cortana_relay.base_url", api_base_url),
            "-c".to_string(),
            codex_cli_config_override("model_providers.cortana_relay.wire_api", "responses"),
            "-c".to_string(),
            codex_cli_config_override(
                "model_providers.cortana_relay.env_key",
                CODEX_RELAY_API_KEY_ENV,
            ),
        ],
    )
}
