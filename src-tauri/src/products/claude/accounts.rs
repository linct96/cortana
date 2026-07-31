use crate::{
    features::{
        accounts::{get_profile_summary_for_product, list_profiles_for_product, relay_alias},
        models,
    },
    platform::{
        config,
        db::{credential_fingerprint, database_error, open_database},
        files::write_file_atomically,
        local_web,
        state::{
            now_millis, AccountProduct, AppState, AppStatus, AuthState, ConfigDiagnostic,
            ConfigFile, ProfileSummary, ACCOUNT_TYPE_OAUTH, ACCOUNT_TYPE_RELAY,
        },
    },
    products::codex::normalize_api_base_url,
};
use reqwest::blocking::Client;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use tauri::State;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use uuid::Uuid;

pub(crate) const CLAUDE_OAUTH_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
pub(crate) const CLAUDE_OAUTH_TOKEN_URL: &str = "https://api.anthropic.com/v1/oauth/token";
pub(crate) const CLAUDE_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub(crate) const CLAUDE_OAUTH_CALLBACK_URL: &str = "http://localhost:54545/callback";
pub(crate) const CLAUDE_OAUTH_SCOPE: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

const CLAUDE_CLI_TOKEN_SECONDS: i64 = 31_536_000;
const RENEWAL_LEAD_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;
static SWITCH_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClaudeOAuthTokenResponse {
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: String,
    #[serde(default)]
    pub(crate) expires_in: Option<i64>,
    #[serde(default)]
    account: Option<ClaudeAccount>,
    #[serde(default)]
    organization: Option<ClaudeOrganization>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeAccount {
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    email_address: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeOrganization {
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum ClaudeCredential {
    #[serde(rename = "claude_oauth")]
    OAuth {
        #[serde(default, rename = "refreshToken")]
        refresh_token: String,
        #[serde(rename = "cliToken")]
        cli_token: String,
        #[serde(default, rename = "cliTokenExpiresAt")]
        cli_token_expires_at: Option<i64>,
        #[serde(default = "default_scope", rename = "scopes")]
        scopes: String,
        #[serde(default)]
        organization: Option<ClaudeOrganization>,
        source: String,
    },
    #[serde(rename = "claude_relay")]
    Relay {
        #[serde(rename = "authToken")]
        auth_token: String,
        source: String,
    },
}

enum SettingsCredential {
    Missing,
    OAuth {
        token: String,
    },
    Relay {
        api_base_url: String,
        auth_token: String,
    },
    Unmanaged(String),
}

fn default_scope() -> String {
    CLAUDE_OAUTH_SCOPE.to_string()
}

pub(crate) fn app_status(app: &tauri::AppHandle, state: &AppState) -> Result<AppStatus, String> {
    let connection = open_database(state)?;
    let path = settings_path(state);
    let settings = read_settings(&path)?;
    let current = settings_credential(&settings);
    let active_id = profile_id_for_settings_credential(&connection, &current)?;
    let profiles =
        list_profiles_for_product(&connection, AccountProduct::Claude, active_id.as_deref())?;
    let (kind, message, detected_profile) = match current {
        SettingsCredential::Missing => (
            "missing",
            "尚未检测到 Claude OAuth 或中转站凭据。".to_string(),
            None,
        ),
        SettingsCredential::OAuth { .. } if active_id.is_some() => (
            "managed",
            "当前 Claude 登录状态已匹配已保存账户。".to_string(),
            None,
        ),
        SettingsCredential::OAuth { ref token } => (
            "unmanaged",
            "当前 Claude OAuth Token 尚未纳入本应用管理。".to_string(),
            Some(detected_oauth_profile(token)),
        ),
        SettingsCredential::Relay { .. } if active_id.is_some() => (
            "managed",
            "当前 Claude 中转站配置已匹配已保存账户。".to_string(),
            None,
        ),
        SettingsCredential::Relay {
            ref api_base_url,
            ref auth_token,
        } => (
            "unmanaged",
            "当前 Claude 中转站配置尚未纳入本应用管理。".to_string(),
            Some(detected_relay_profile(api_base_url, auth_token)),
        ),
        SettingsCredential::Unmanaged(message) => ("unmanaged", message, None),
    };
    Ok(AppStatus {
        profiles,
        detected_profile,
        auth_path: path.display().to_string(),
        auth_state: AuthState {
            kind: kind.to_string(),
            message,
        },
        autostart_enabled: app.autolaunch().is_enabled().unwrap_or(false),
        web_access: local_web::web_access_status(app, state)?,
    })
}

pub(crate) fn exchange_code(
    code: &str,
    state: &str,
    code_verifier: &str,
    callback_url: &str,
) -> Result<ClaudeOAuthTokenResponse, String> {
    request_token(
        &json!({
            "code": code,
            "state": state,
            "grant_type": "authorization_code",
            "client_id": CLAUDE_OAUTH_CLIENT_ID,
            "redirect_uri": callback_url,
            "code_verifier": code_verifier,
        }),
        "Claude OAuth code exchange",
    )
}

pub(crate) fn upsert_oauth_profile(
    state: &AppState,
    token: ClaudeOAuthTokenResponse,
    requested_alias: &str,
) -> Result<ProfileSummary, String> {
    if token.refresh_token.trim().is_empty() {
        return Err("Claude OAuth 未返回 refresh_token，无法保存可续期凭据。".to_string());
    }
    let cli_token = mint_cli_token(&token.refresh_token)?;
    let account = token.account.unwrap_or_default();
    let account_id =
        non_empty(&account.uuid).unwrap_or_else(|| credential_fingerprint(&cli_token.access_token));
    let credential = ClaudeCredential::OAuth {
        refresh_token: if cli_token.refresh_token.trim().is_empty() {
            token.refresh_token
        } else {
            cli_token.refresh_token
        },
        cli_token: cli_token.access_token,
        cli_token_expires_at: expires_at(cli_token.expires_in),
        scopes: CLAUDE_OAUTH_SCOPE.to_string(),
        organization: token.organization,
        source: "oauth".to_string(),
    };
    upsert_credential(
        state,
        &credential,
        &account_id,
        account.email_address.trim(),
        requested_alias,
    )
}

pub(crate) fn import_current_profile(
    state: &AppState,
    requested_alias: Option<String>,
) -> Result<ProfileSummary, String> {
    match settings_credential(&read_settings(&settings_path(state))?) {
        SettingsCredential::OAuth { token } => {
            let credential = ClaudeCredential::OAuth {
                refresh_token: String::new(),
                cli_token: token.clone(),
                cli_token_expires_at: None,
                scopes: CLAUDE_OAUTH_SCOPE.to_string(),
                organization: None,
                source: "settings_import".to_string(),
            };
            upsert_credential(
                state,
                &credential,
                &credential_fingerprint(&token),
                "",
                requested_alias.as_deref().unwrap_or_default(),
            )
        }
        SettingsCredential::Relay {
            api_base_url,
            auth_token,
        } => upsert_relay_profile(
            state,
            &auth_token,
            &api_base_url,
            requested_alias.as_deref().unwrap_or_default(),
            "settings_import",
        ),
        SettingsCredential::Missing => Err("未找到 Claude OAuth 或中转站凭据。".to_string()),
        SettingsCredential::Unmanaged(message) => Err(message),
    }
}

pub(crate) fn switch_profile(
    state: &AppState,
    profile_id: &str,
    force: bool,
) -> Result<ProfileSummary, String> {
    let _guard = SWITCH_LOCK
        .try_lock()
        .map_err(|_| "已有 Claude 账号切换正在进行，请稍后重试。".to_string())?;
    let connection = open_database(state)?;
    let path = settings_path(state);
    let settings = read_settings(&path)?;
    let current = settings_credential(&settings);
    let active_id = profile_id_for_settings_credential(&connection, &current)?;
    if active_id.is_none() && !matches!(current, SettingsCredential::Missing) && !force {
        return Err(
            "检测到工具外的 Claude 登录变更。请先同步当前账号，或确认后强制切换。".to_string(),
        );
    }
    let mut credential = load_credential(&connection, profile_id)?;
    refresh_credential_if_needed(&mut credential)?;
    let (model_profile_id, default_model_id) = connection
        .query_row(
            "SELECT model_profile_id, default_model_id FROM accounts WHERE id = ?1 AND product = 'claude'",
            params![profile_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .map_err(database_error)?;
    write_profile_to_settings(
        state,
        &path,
        &credential,
        relay_api_base_url(&connection, profile_id, &credential)?.as_deref(),
        model_profile_id.as_deref(),
        default_model_id.as_deref(),
    )?;
    save_credential(&connection, profile_id, &credential, true)?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Claude,
        profile_id,
        Some(profile_id),
    )
}

pub(crate) fn update_alias(
    state: &AppState,
    profile_id: &str,
    alias: &str,
) -> Result<ProfileSummary, String> {
    let alias = alias.trim();
    if alias.is_empty() {
        return Err("别名不能为空。".to_string());
    }
    let connection = open_database(state)?;
    if connection
        .execute(
            "UPDATE accounts SET alias = ?1, updated_at = ?2 WHERE id = ?3 AND product = 'claude'",
            params![alias, now_millis(), profile_id],
        )
        .map_err(database_error)?
        == 0
    {
        return Err("账户不存在。".to_string());
    }
    let active_id = profile_id_for_settings_credential(
        &connection,
        &settings_credential(&read_settings(&settings_path(state))?),
    )?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Claude,
        profile_id,
        active_id.as_deref(),
    )
}

pub(crate) fn refresh_profile_usage(
    state: &AppState,
    profile_id: &str,
) -> Result<ProfileSummary, String> {
    let connection = open_database(state)?;
    let mut credential = load_credential(&connection, profile_id)?;
    if !matches!(credential, ClaudeCredential::OAuth { .. }) {
        return Err("中转站账户不支持登录令牌更新。".to_string());
    }
    let ClaudeCredential::OAuth { refresh_token, .. } = &credential else {
        unreachable!();
    };
    if refresh_token.trim().is_empty() {
        return Err(
            "此 Claude Token 从 settings.json 导入，无法续期，请重新进行浏览器授权。".to_string(),
        );
    }
    refresh_credential(&mut credential)?;
    let path = settings_path(state);
    let active_id = profile_id_for_settings_credential(
        &connection,
        &settings_credential(&read_settings(&path)?),
    )?;
    if active_id.as_deref() == Some(profile_id) {
        write_credential_to_settings(&path, &credential, None)?;
    }
    save_credential(&connection, profile_id, &credential, false)?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Claude,
        profile_id,
        active_id.as_deref(),
    )
}

pub(crate) fn clear_active_profile(state: &AppState, profile_id: &str) -> Result<(), String> {
    let connection = open_database(state)?;
    let credential = load_credential(&connection, profile_id)?;
    let path = settings_path(state);
    let mut settings = read_settings(&path)?;
    let relay_api_base_url = relay_api_base_url(&connection, profile_id, &credential)?;
    if !credential_matches_settings(
        &credential,
        &settings_credential(&settings),
        relay_api_base_url.as_deref(),
    ) {
        return Ok(());
    }
    let environment = environment_mut(&mut settings)?;
    clear_credential_from_environment(environment, &credential);
    models::apply_claude_model_config(state, &mut settings, None, None)?;
    write_settings(&path, &settings)
}

pub(crate) fn credential_is_renewable(auth_json: &str) -> bool {
    serde_json::from_str::<ClaudeCredential>(auth_json)
        .map(|credential| {
            matches!(credential, ClaudeCredential::OAuth { refresh_token, .. } if !refresh_token.trim().is_empty())
        })
        .unwrap_or(false)
}

fn mint_cli_token(refresh_token: &str) -> Result<ClaudeOAuthTokenResponse, String> {
    request_token(
        &json!({
            "client_id": CLAUDE_OAUTH_CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "expires_in": CLAUDE_CLI_TOKEN_SECONDS,
            "scope": CLAUDE_OAUTH_SCOPE,
        }),
        "Claude OAuth token 刷新",
    )
}

fn request_token(payload: &Value, label: &str) -> Result<ClaudeOAuthTokenResponse, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(CLAUDE_OAUTH_TOKEN_URL)
        .header("Accept", "application/json")
        .json(payload)
        .send()
        .map_err(|error| format!("{label} 请求失败：{error}"))?;
    let status = response.status();
    let body = response.text().map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!("{label} 失败（HTTP {}）。", status.as_u16()));
    }
    let token: ClaudeOAuthTokenResponse =
        serde_json::from_str(&body).map_err(|_| format!("{label} 响应不是有效 JSON。"))?;
    if token.access_token.trim().is_empty() {
        return Err(format!("{label} 未返回 access_token。"));
    }
    Ok(token)
}

fn upsert_credential(
    state: &AppState,
    credential: &ClaudeCredential,
    account_id: &str,
    email: &str,
    requested_alias: &str,
) -> Result<ProfileSummary, String> {
    let ClaudeCredential::OAuth { cli_token, .. } = credential else {
        return Err("Claude OAuth 凭据格式无效。".to_string());
    };
    let mut connection = open_database(state)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let existing = profile_id_for_cli_token(&transaction, cli_token)?.or(
        transaction
            .query_row(
                "SELECT id FROM accounts WHERE product = 'claude' AND (account_id = ?1 OR (?2 <> '' AND email = ?2 COLLATE NOCASE)) LIMIT 1",
                params![account_id, email],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?,
    );
    let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
    let auth_json = serde_json::to_string_pretty(credential).map_err(|error| error.to_string())?;
    let now = now_millis();
    if transaction
        .execute(
            "UPDATE accounts SET account_type = 'oauth', api_base_url = NULL, account_id = ?1, email = ?2, alias = CASE WHEN ?3 = '' THEN alias ELSE ?3 END, auth_json = ?4, updated_at = ?5 WHERE id = ?6 AND product = 'claude'",
            params![account_id, email, requested_alias.trim(), auth_json, now, id],
        )
        .map_err(database_error)?
        == 0
    {
        let alias = non_empty(requested_alias)
            .or_else(|| non_empty(email))
            .unwrap_or_else(|| "导入的 Claude Token".to_string());
        transaction
            .execute(
                "INSERT INTO accounts (id, product, account_type, account_id, email, alias, plan_type, auth_json, created_at, updated_at, sort_order) VALUES (?1, 'claude', 'oauth', ?2, ?3, ?4, '', ?5, ?6, ?6, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'claude'), 0))",
                params![id, account_id, email, alias, auth_json, now],
            )
            .map_err(database_error)?;
    }
    transaction.commit().map_err(database_error)?;
    let active_id = profile_id_for_settings_credential(
        &connection,
        &settings_credential(&read_settings(&settings_path(state))?),
    )?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Claude,
        &id,
        active_id.as_deref(),
    )
}

pub(crate) fn upsert_relay_profile(
    state: &AppState,
    auth_token: &str,
    api_base_url: &str,
    requested_alias: &str,
    source: &str,
) -> Result<ProfileSummary, String> {
    let api_base_url = normalize_api_base_url(api_base_url)?;
    let auth_token = non_empty(auth_token).ok_or_else(|| "API Key 不能为空。".to_string())?;
    let credential = ClaudeCredential::Relay {
        auth_token: auth_token.clone(),
        source: source.to_string(),
    };
    let auth_json = serde_json::to_string_pretty(&credential).map_err(|error| error.to_string())?;
    let fingerprint = credential_fingerprint(&auth_token);
    let mut connection = open_database(state)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let existing = transaction
        .query_row(
            "SELECT id, alias FROM accounts WHERE product = 'claude' AND account_type = 'relay' AND api_base_url = ?1 AND account_id = ?2 LIMIT 1",
            params![api_base_url, fingerprint],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    let now = now_millis();
    let id = if let Some((id, current_alias)) = existing {
        let alias = if requested_alias.trim().is_empty() {
            current_alias
        } else {
            requested_alias.trim().to_string()
        };
        transaction
            .execute(
                "UPDATE accounts SET alias = ?1, auth_json = ?2, api_base_url = ?3, account_id = ?4, updated_at = ?5 WHERE id = ?6 AND product = 'claude'",
                params![alias, auth_json, api_base_url, fingerprint, now, id],
            )
            .map_err(database_error)?;
        id
    } else {
        let id = Uuid::new_v4().to_string();
        let alias = relay_alias(requested_alias, &api_base_url);
        transaction
            .execute(
                "INSERT INTO accounts (id, product, account_type, api_base_url, account_id, email, alias, plan_type, auth_json, created_at, updated_at, last_used_at, sort_order) VALUES (?1, 'claude', 'relay', ?2, ?3, '', ?4, '', ?5, ?6, ?6, NULL, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'claude'), 0))",
                params![id, api_base_url, fingerprint, alias, auth_json, now],
            )
            .map_err(database_error)?;
        id
    };
    transaction.commit().map_err(database_error)?;
    let active_id = profile_id_for_settings_credential(
        &connection,
        &settings_credential(&read_settings(&settings_path(state))?),
    )?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Claude,
        &id,
        active_id.as_deref(),
    )
}

pub(crate) fn update_relay_profile(
    state: &AppState,
    profile_id: &str,
    requested_alias: &str,
    requested_auth_token: Option<&str>,
    requested_api_base_url: &str,
) -> Result<ProfileSummary, String> {
    let api_base_url = normalize_api_base_url(requested_api_base_url)?;
    let mut connection = open_database(state)?;
    let (account_type, auth_json) = connection
        .query_row(
            "SELECT account_type, auth_json FROM accounts WHERE id = ?1 AND product = 'claude'",
            params![profile_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type != ACCOUNT_TYPE_RELAY {
        return Err("该账户不是中转站账户。".to_string());
    }
    let stored = serde_json::from_str::<ClaudeCredential>(&auth_json)
        .map_err(|_| "存档的 Claude 凭据已损坏。".to_string())?;
    let existing_auth_token = match stored {
        ClaudeCredential::Relay { auth_token, .. } => auth_token,
        ClaudeCredential::OAuth { .. } => {
            return Err("存档的 Claude 中转站凭据已损坏。".to_string())
        }
    };
    let auth_token = requested_auth_token
        .and_then(non_empty)
        .unwrap_or(existing_auth_token);
    let fingerprint = credential_fingerprint(&auth_token);
    let alias = relay_alias(requested_alias, &api_base_url);
    let credential = ClaudeCredential::Relay {
        auth_token,
        source: "manual".to_string(),
    };
    let active_id = profile_id_for_settings_credential(
        &connection,
        &settings_credential(&read_settings(&settings_path(state))?),
    )?;
    let active = active_id.as_deref() == Some(profile_id);
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let duplicate_exists = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE product = 'claude' AND account_type = 'relay' AND id <> ?1 AND api_base_url = ?2 AND account_id = ?3)",
            params![profile_id, api_base_url, fingerprint],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if duplicate_exists {
        return Err("已存在使用相同 API Key 和地址的中转站账户。".to_string());
    }
    if active {
        write_credential_to_settings(&settings_path(state), &credential, Some(&api_base_url))?;
    }
    save_credential(&transaction, profile_id, &credential, false)?;
    transaction
        .execute(
            "UPDATE accounts SET alias = ?1, api_base_url = ?2, account_id = ?3 WHERE id = ?4 AND product = 'claude'",
            params![alias, api_base_url, fingerprint, profile_id],
        )
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Claude,
        profile_id,
        active.then_some(profile_id),
    )
}

fn load_credential(connection: &Connection, profile_id: &str) -> Result<ClaudeCredential, String> {
    let auth_json = connection
        .query_row(
            "SELECT auth_json FROM accounts WHERE id = ?1 AND product = 'claude'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    serde_json::from_str(&auth_json).map_err(|_| "存档的 Claude 凭据已损坏。".to_string())
}

fn save_credential(
    connection: &Connection,
    profile_id: &str,
    credential: &ClaudeCredential,
    used: bool,
) -> Result<(), String> {
    let now = now_millis();
    let auth_json = serde_json::to_string_pretty(credential).map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE accounts SET auth_json = ?1, last_used_at = CASE WHEN ?2 THEN ?3 ELSE last_used_at END, updated_at = ?3 WHERE id = ?4 AND product = 'claude'",
            params![auth_json, used, now, profile_id],
        )
        .map_err(database_error)?;
    Ok(())
}

fn refresh_credential_if_needed(credential: &mut ClaudeCredential) -> Result<(), String> {
    let ClaudeCredential::OAuth {
        refresh_token,
        cli_token_expires_at,
        ..
    } = credential
    else {
        return Ok(());
    };
    if refresh_token.trim().is_empty() || !needs_renewal(*cli_token_expires_at) {
        return Ok(());
    }
    refresh_credential(credential)
}

fn refresh_credential(credential: &mut ClaudeCredential) -> Result<(), String> {
    let ClaudeCredential::OAuth {
        refresh_token,
        cli_token,
        cli_token_expires_at,
        ..
    } = credential
    else {
        return Err("中转站账户不支持登录令牌更新。".to_string());
    };
    let token = mint_cli_token(refresh_token)?;
    *cli_token = token.access_token;
    if !token.refresh_token.trim().is_empty() {
        *refresh_token = token.refresh_token;
    }
    *cli_token_expires_at = expires_at(token.expires_in);
    Ok(())
}

fn needs_renewal(expires_at: Option<i64>) -> bool {
    expires_at
        .is_some_and(|expires_at| expires_at <= now_millis().saturating_add(RENEWAL_LEAD_MILLIS))
}

pub(crate) fn settings_path(state: &AppState) -> PathBuf {
    state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home)
        .join(".claude/settings.json")
}

pub(crate) fn get_claude_config(state: State<'_, AppState>) -> Result<ConfigFile, String> {
    read_claude_config(&state)
}

fn read_claude_config(state: &AppState) -> Result<ConfigFile, String> {
    config::read_config(&settings_path(state), "{}", "Claude settings.json")
}

pub(crate) fn save_claude_config(
    state: State<'_, AppState>,
    content: String,
) -> Result<(), String> {
    save_claude_config_internal(&state, &content)
}

fn save_claude_config_internal(state: &AppState, content: &str) -> Result<(), String> {
    parse_claude_config(content)?;
    write_file_atomically(&settings_path(state), content)
}

pub(crate) fn validate_claude_config(content: String) -> Vec<ConfigDiagnostic> {
    config::validate_json_object(&content, "Claude settings.json")
}

pub(crate) fn format_claude_config(content: String) -> Result<String, String> {
    config::format_json_object(&content, "Claude settings.json")
}

fn parse_claude_config(content: &str) -> Result<Value, String> {
    config::parse_json_object(content, "Claude settings.json")
}

pub(crate) fn read_settings(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("无法读取 Claude settings.json：{error}"))?;
    if content.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(&content)
        .map_err(|_| "Claude settings.json 不是有效的 JSON。".to_string())?;
    value
        .is_object()
        .then_some(value)
        .ok_or_else(|| "Claude settings.json 必须是一个 JSON 对象。".to_string())
}

fn write_settings(path: &Path, settings: &Value) -> Result<(), String> {
    write_file_atomically(
        path,
        &serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?,
    )
}

fn oauth_token_from_settings(settings: &Value) -> Option<String> {
    environment_value(settings, "CLAUDE_CODE_OAUTH_TOKEN")
}

fn environment_mut(settings: &mut Value) -> Result<&mut serde_json::Map<String, Value>, String> {
    let root = settings
        .as_object_mut()
        .ok_or_else(|| "Claude settings.json 必须是一个 JSON 对象。".to_string())?;
    let env = root.entry("env".to_string()).or_insert_with(|| json!({}));
    env.as_object_mut()
        .ok_or_else(|| "Claude settings.json 的 env 必须是一个 JSON 对象。".to_string())
}

fn environment_value(settings: &Value, key: &str) -> Option<String> {
    settings
        .get("env")
        .and_then(Value::as_object)
        .and_then(|env| env.get(key))
        .and_then(Value::as_str)
        .and_then(non_empty)
}

fn settings_credential(settings: &Value) -> SettingsCredential {
    let oauth_token = oauth_token_from_settings(settings);
    let api_base_url = environment_value(settings, "ANTHROPIC_BASE_URL");
    let auth_token = environment_value(settings, "ANTHROPIC_AUTH_TOKEN");
    let api_key = environment_value(settings, "ANTHROPIC_API_KEY");
    let has_relay_value = api_base_url.is_some() || auth_token.is_some();

    if oauth_token.is_some() && (has_relay_value || api_key.is_some()) {
        return SettingsCredential::Unmanaged(
            "检测到 Claude OAuth 与 API 凭据同时存在，请确认后再覆盖。".to_string(),
        );
    }
    if api_key.is_some() {
        return SettingsCredential::Unmanaged(
            "检测到 ANTHROPIC_API_KEY。当前仅支持使用 ANTHROPIC_AUTH_TOKEN 的中转站账户。"
                .to_string(),
        );
    }
    if has_relay_value {
        let (Some(api_base_url), Some(auth_token)) = (api_base_url, auth_token) else {
            return SettingsCredential::Unmanaged(
                "Claude 中转站配置不完整，需要同时设置 ANTHROPIC_BASE_URL 与 ANTHROPIC_AUTH_TOKEN。"
                    .to_string(),
            );
        };
        return match normalize_api_base_url(&api_base_url) {
            Ok(api_base_url) => SettingsCredential::Relay {
                api_base_url,
                auth_token,
            },
            Err(_) => SettingsCredential::Unmanaged(
                "ANTHROPIC_BASE_URL 不是有效的 HTTP(S) 地址。".to_string(),
            ),
        };
    }
    oauth_token.map_or(SettingsCredential::Missing, |token| {
        SettingsCredential::OAuth { token }
    })
}

fn write_credential_to_settings(
    path: &Path,
    credential: &ClaudeCredential,
    relay_api_base_url: Option<&str>,
) -> Result<(), String> {
    let mut settings = read_settings(path)?;
    apply_credential_to_settings(&mut settings, credential, relay_api_base_url)?;
    write_settings(path, &settings)
}

fn write_profile_to_settings(
    state: &AppState,
    path: &Path,
    credential: &ClaudeCredential,
    relay_api_base_url: Option<&str>,
    model_profile_id: Option<&str>,
    default_model_id: Option<&str>,
) -> Result<(), String> {
    let mut settings = read_settings(path)?;
    apply_credential_to_settings(&mut settings, credential, relay_api_base_url)?;
    models::apply_claude_model_config(state, &mut settings, model_profile_id, default_model_id)?;
    write_settings(path, &settings)
}

fn apply_credential_to_settings(
    settings: &mut Value,
    credential: &ClaudeCredential,
    relay_api_base_url: Option<&str>,
) -> Result<(), String> {
    let environment = environment_mut(settings)?;
    match credential {
        ClaudeCredential::OAuth {
            cli_token, scopes, ..
        } => {
            clear_relay_environment(environment);
            environment.insert(
                "CLAUDE_CODE_OAUTH_TOKEN".to_string(),
                Value::String(cli_token.clone()),
            );
            environment.insert(
                "CLAUDE_CODE_OAUTH_SCOPES".to_string(),
                Value::String(scopes.clone()),
            );
        }
        ClaudeCredential::Relay { auth_token, .. } => {
            let api_base_url =
                relay_api_base_url.ok_or_else(|| "中转站账户缺少 API 地址。".to_string())?;
            clear_oauth_environment(environment);
            clear_relay_environment(environment);
            environment.insert(
                "ANTHROPIC_BASE_URL".to_string(),
                Value::String(api_base_url.to_string()),
            );
            environment.insert(
                "ANTHROPIC_AUTH_TOKEN".to_string(),
                Value::String(auth_token.clone()),
            );
        }
    }
    Ok(())
}

fn clear_credential_from_environment(
    environment: &mut serde_json::Map<String, Value>,
    credential: &ClaudeCredential,
) {
    match credential {
        ClaudeCredential::OAuth { .. } => clear_oauth_environment(environment),
        ClaudeCredential::Relay { .. } => clear_relay_environment(environment),
    }
}

fn clear_oauth_environment(environment: &mut serde_json::Map<String, Value>) {
    environment.remove("CLAUDE_CODE_OAUTH_TOKEN");
    environment.remove("CLAUDE_CODE_OAUTH_SCOPES");
}

fn clear_relay_environment(environment: &mut serde_json::Map<String, Value>) {
    environment.remove("ANTHROPIC_BASE_URL");
    environment.remove("ANTHROPIC_AUTH_TOKEN");
    environment.remove("ANTHROPIC_API_KEY");
}

fn profile_id_for_cli_token(
    connection: &Connection,
    token: &str,
) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT id FROM accounts WHERE product = 'claude' AND json_extract(auth_json, '$.cliToken') = ?1 LIMIT 1",
            params![token],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)
}

fn profile_id_for_relay(
    connection: &Connection,
    api_base_url: &str,
    auth_token: &str,
) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT id FROM accounts WHERE product = 'claude' AND account_type = 'relay' AND api_base_url = ?1 AND account_id = ?2 LIMIT 1",
            params![api_base_url, credential_fingerprint(auth_token)],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)
}

fn profile_id_for_settings_credential(
    connection: &Connection,
    credential: &SettingsCredential,
) -> Result<Option<String>, String> {
    match credential {
        SettingsCredential::OAuth { token } => profile_id_for_cli_token(connection, token),
        SettingsCredential::Relay {
            api_base_url,
            auth_token,
        } => profile_id_for_relay(connection, api_base_url, auth_token),
        SettingsCredential::Missing | SettingsCredential::Unmanaged(_) => Ok(None),
    }
}

fn relay_api_base_url(
    connection: &Connection,
    profile_id: &str,
    credential: &ClaudeCredential,
) -> Result<Option<String>, String> {
    if !matches!(credential, ClaudeCredential::Relay { .. }) {
        return Ok(None);
    }
    let api_base_url = connection
        .query_row(
            "SELECT api_base_url FROM accounts WHERE id = ?1 AND product = 'claude' AND account_type = 'relay'",
            params![profile_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(database_error)?
        .flatten()
        .ok_or_else(|| "中转站账户缺少 API 地址。".to_string())?;
    normalize_api_base_url(&api_base_url).map(Some)
}

fn credential_matches_settings(
    credential: &ClaudeCredential,
    current: &SettingsCredential,
    relay_api_base_url: Option<&str>,
) -> bool {
    match (credential, current) {
        (ClaudeCredential::OAuth { cli_token, .. }, SettingsCredential::OAuth { token }) => {
            cli_token == token
        }
        (
            ClaudeCredential::Relay { auth_token, .. },
            SettingsCredential::Relay {
                api_base_url,
                auth_token: current,
            },
        ) => auth_token == current && relay_api_base_url == Some(api_base_url),
        _ => false,
    }
}

fn detected_oauth_profile(token: &str) -> ProfileSummary {
    ProfileSummary {
        id: "detected".to_string(),
        product: AccountProduct::Claude,
        account_type: ACCOUNT_TYPE_OAUTH.to_string(),
        api_base_url: None,
        account_id: credential_fingerprint(token),
        email: String::new(),
        alias: "当前 Claude Token".to_string(),
        plan_type: String::new(),
        usage_primary: None,
        usage_secondary: None,
        antigravity_quota: None,
        usage_updated_at: None,
        reset_credits_available_count: None,
        needs_reauthorization: false,
        is_renewable: false,
        is_active: true,
        last_used_at: None,
        updated_at: now_millis(),
    }
}

fn detected_relay_profile(api_base_url: &str, auth_token: &str) -> ProfileSummary {
    ProfileSummary {
        id: "detected".to_string(),
        product: AccountProduct::Claude,
        account_type: ACCOUNT_TYPE_RELAY.to_string(),
        api_base_url: Some(api_base_url.to_string()),
        account_id: credential_fingerprint(auth_token),
        email: String::new(),
        alias: relay_alias("", api_base_url),
        plan_type: String::new(),
        usage_primary: None,
        usage_secondary: None,
        antigravity_quota: None,
        usage_updated_at: None,
        reset_credits_available_count: None,
        needs_reauthorization: false,
        is_renewable: false,
        is_active: true,
        last_used_at: None,
        updated_at: now_millis(),
    }
}

fn expires_at(expires_in: Option<i64>) -> Option<i64> {
    let seconds = expires_in.unwrap_or(CLAUDE_CLI_TOKEN_SECONDS);
    (seconds > 0).then(|| now_millis().saturating_add(seconds.saturating_mul(1000)))
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        clear_active_profile, format_claude_config, import_current_profile,
        oauth_token_from_settings, read_claude_config, read_settings, save_claude_config_internal,
        settings_path, switch_profile, update_relay_profile, upsert_credential,
        write_credential_to_settings, write_settings, ClaudeCredential, CLAUDE_OAUTH_SCOPE,
    };
    use crate::{
        features::accounts::relay_api_key_for_profile,
        platform::{
            db::{credential_fingerprint, initialize_database, open_database},
            files::write_file_atomically,
            state::{AccountProduct, AppState, ACCOUNT_TYPE_RELAY},
        },
    };
    use serde_json::json;
    use std::{
        fs,
        sync::{Arc, Mutex},
    };
    use uuid::Uuid;

    static TEST_SWITCH_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn reads_formats_and_safely_saves_claude_config() {
        let directory = std::env::temp_dir().join(format!("cortana-claude-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();

        let initial = read_claude_config(&state).unwrap();
        assert_eq!(
            initial.path,
            directory
                .join(".claude/settings.json")
                .display()
                .to_string()
        );
        assert_eq!(initial.content, "{}");

        let formatted = format_claude_config(
            r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"secret"},"model":"opus"}"#.into(),
        )
        .unwrap();
        save_claude_config_internal(&state, &formatted).unwrap();
        assert_eq!(read_claude_config(&state).unwrap().content, formatted);

        assert!(save_claude_config_internal(&state, "[]").is_err());
        assert_eq!(
            fs::read_to_string(settings_path(&state)).unwrap(),
            formatted
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn keeps_unrelated_claude_settings_when_switching_token() {
        let directory = std::env::temp_dir().join(format!("cortana-claude-{}", Uuid::new_v4()));
        let path = directory.join(".claude/settings.json");
        write_file_atomically(
            &path,
            r#"{"permissions":{"allow":["Bash"]},"env":{"KEEP":"yes"}}"#,
        )
        .unwrap();
        let credential = ClaudeCredential::OAuth {
            refresh_token: "refresh".to_string(),
            cli_token: "token".to_string(),
            cli_token_expires_at: None,
            scopes: CLAUDE_OAUTH_SCOPE.to_string(),
            organization: None,
            source: "oauth".to_string(),
        };

        write_credential_to_settings(&path, &credential, None).unwrap();

        let settings = read_settings(&path).unwrap();
        assert_eq!(settings["permissions"]["allow"][0], "Bash");
        assert_eq!(settings["env"]["KEEP"], "yes");
        assert_eq!(settings["env"]["CLAUDE_CODE_OAUTH_TOKEN"], "token");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn imports_and_switches_a_relay_without_touching_unrelated_settings() {
        let _guard = TEST_SWITCH_LOCK.lock().unwrap();
        let directory = std::env::temp_dir().join(format!("cortana-claude-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        write_file_atomically(
            &settings_path(&state),
            r#"{"permissions":{"allow":["Bash"]},"env":{"KEEP":"yes","ANTHROPIC_BASE_URL":"https://relay.example.com/v1/","ANTHROPIC_AUTH_TOKEN":"relay-token"}}"#,
        )
        .unwrap();

        let relay = import_current_profile(&state, None).unwrap();
        assert_eq!(relay.account_type, ACCOUNT_TYPE_RELAY);
        assert!(relay.is_active);
        assert_eq!(
            relay.api_base_url.as_deref(),
            Some("https://relay.example.com/v1")
        );
        let connection = open_database(&state).unwrap();
        assert_eq!(
            relay_api_key_for_profile(&connection, &relay.id, AccountProduct::Claude).unwrap(),
            "relay-token"
        );
        drop(connection);

        let oauth = ClaudeCredential::OAuth {
            refresh_token: String::new(),
            cli_token: "oauth-token".to_string(),
            cli_token_expires_at: None,
            scopes: CLAUDE_OAUTH_SCOPE.to_string(),
            organization: None,
            source: "settings_import".to_string(),
        };
        let oauth = upsert_credential(
            &state,
            &oauth,
            &credential_fingerprint("oauth-token"),
            "",
            "OAuth",
        )
        .unwrap();

        switch_profile(&state, &oauth.id, false).unwrap();
        let settings = read_settings(&settings_path(&state)).unwrap();
        assert_eq!(settings["permissions"]["allow"][0], "Bash");
        assert_eq!(settings["env"]["KEEP"], "yes");
        assert_eq!(settings["env"]["CLAUDE_CODE_OAUTH_TOKEN"], "oauth-token");
        assert!(settings["env"].get("ANTHROPIC_BASE_URL").is_none());
        assert!(settings["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());

        let mut settings = settings;
        settings["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"] = json!("external-fable");
        write_settings(&settings_path(&state), &settings).unwrap();
        switch_profile(&state, &relay.id, false).unwrap();
        let settings = read_settings(&settings_path(&state)).unwrap();
        assert_eq!(settings["env"]["KEEP"], "yes");
        assert_eq!(
            settings["env"]["ANTHROPIC_BASE_URL"],
            "https://relay.example.com/v1"
        );
        assert_eq!(settings["env"]["ANTHROPIC_AUTH_TOKEN"], "relay-token");
        assert!(settings["env"]
            .get("ANTHROPIC_DEFAULT_FABLE_MODEL")
            .is_none());
        assert!(settings["env"].get("CLAUDE_CODE_OAUTH_TOKEN").is_none());

        let updated = update_relay_profile(
            &state,
            &relay.id,
            "编辑后的中转站",
            Some("updated-token"),
            "https://relay.example.com/v2",
        )
        .unwrap();
        assert!(updated.is_active);
        assert_eq!(updated.alias, "编辑后的中转站");
        assert_eq!(
            updated.api_base_url.as_deref(),
            Some("https://relay.example.com/v2")
        );
        let settings = read_settings(&settings_path(&state)).unwrap();
        assert_eq!(
            settings["env"]["ANTHROPIC_BASE_URL"],
            "https://relay.example.com/v2"
        );
        assert_eq!(settings["env"]["ANTHROPIC_AUTH_TOKEN"], "updated-token");

        clear_active_profile(&state, &relay.id).unwrap();
        let settings = read_settings(&settings_path(&state)).unwrap();
        assert_eq!(settings["env"]["KEEP"], "yes");
        assert!(settings["env"].get("ANTHROPIC_BASE_URL").is_none());
        assert!(settings["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn requires_confirmation_before_replacing_an_unmanaged_token() {
        let _guard = TEST_SWITCH_LOCK.lock().unwrap();
        let directory = std::env::temp_dir().join(format!("cortana-claude-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        write_file_atomically(
            &settings_path(&state),
            r#"{"env":{"CLAUDE_CODE_OAUTH_TOKEN":"external"}}"#,
        )
        .unwrap();
        let credential = ClaudeCredential::OAuth {
            refresh_token: String::new(),
            cli_token: "saved".to_string(),
            cli_token_expires_at: None,
            scopes: CLAUDE_OAUTH_SCOPE.to_string(),
            organization: None,
            source: "settings_import".to_string(),
        };
        let profile = upsert_credential(
            &state,
            &credential,
            &credential_fingerprint("saved"),
            "",
            "saved",
        )
        .unwrap();

        assert!(switch_profile(&state, &profile.id, false)
            .unwrap_err()
            .contains("工具外"));
        switch_profile(&state, &profile.id, true).unwrap();
        assert_eq!(
            oauth_token_from_settings(&read_settings(&settings_path(&state)).unwrap()).as_deref(),
            Some("saved")
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
