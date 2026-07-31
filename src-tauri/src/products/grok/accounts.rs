use crate::{
    features::{
        accounts::{get_profile_summary_for_product, list_profiles_for_product, relay_alias},
        models,
    },
    platform::{
        config,
        db::{credential_fingerprint, database_error, get_setting, open_database, set_setting},
        files::write_file_atomically,
        local_web,
        state::{
            now_millis, AccountProduct, AppState, AppStatus, AuthState, ConfigDiagnostic,
            ConfigFile, ProfileSummary, UsageWindow, ACCOUNT_TYPE_OAUTH, ACCOUNT_TYPE_RELAY,
            MAX_IMPORTED_AUTH_JSON_BYTES,
        },
    },
    products::{
        codex::{normalize_api_base_url, usage::AccountUsage},
        grok::oauth as grok_oauth,
    },
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{SecondsFormat, Utc};
use reqwest::blocking::Client;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use tauri::State;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use toml_edit::{table as toml_table, value as toml_value, DocumentMut};
use uuid::Uuid;

const BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
const ENABLED_RELAY_PROFILES_SETTING: &str = "grok_enabled_relay_profile_ids";
static SWITCH_LOCK: Mutex<()> = Mutex::new(());

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BillingResponse {
    config: Option<BillingConfig>,
    subscription_tier: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BillingConfig {
    credit_usage_percent: Option<f64>,
    current_period: Option<BillingPeriod>,
}

#[derive(Deserialize)]
struct BillingPeriod {
    start: Option<String>,
    end: Option<String>,
}

enum CurrentCredential {
    OAuth(Value),
    Relay,
}

pub(crate) fn app_status(app: &tauri::AppHandle, state: &AppState) -> Result<AppStatus, String> {
    let connection = open_database(state)?;
    let current = current_credential(state)?;
    let enabled_result = enabled_relay_profile_ids(state, &connection);
    let enabled_ids = enabled_result.as_ref().cloned().unwrap_or_default();
    let oauth_active_id = match current.as_ref() {
        Some(CurrentCredential::OAuth(credential)) => {
            profile_id_for_credential(&connection, credential)?
        }
        _ => None,
    };

    if let (Some(profile_id), Some(CurrentCredential::OAuth(credential))) =
        (oauth_active_id.as_deref(), current.as_ref())
    {
        let auth_json =
            serde_json::to_string_pretty(credential).map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE accounts SET auth_json = ?1, updated_at = ?2 WHERE id = ?3 AND product = 'grok' AND auth_json <> ?1",
                params![auth_json, now_millis(), profile_id],
            )
            .map_err(database_error)?;
    }

    if get_setting(&connection, ENABLED_RELAY_PROFILES_SETTING)?.is_none() && enabled_result.is_ok()
    {
        set_enabled_relay_profile_ids(&connection, &enabled_ids)?;
    }
    let mut profiles = list_profiles_for_product(&connection, AccountProduct::Grok, None)?;
    for profile in &mut profiles {
        profile.is_active = if profile.account_type == ACCOUNT_TYPE_RELAY {
            enabled_ids.contains(&profile.id)
        } else {
            oauth_active_id.as_deref() == Some(profile.id.as_str())
        };
    }
    let relay_managed = matches!(current, Some(CurrentCredential::Relay))
        && !enabled_ids.is_empty()
        && relay_config_matches(state, &connection, &enabled_ids)?;
    let (kind, message) = if oauth_active_id.is_some() || relay_managed {
        if relay_managed {
            ("managed", "当前 Grok 中转配置已匹配全部已启用账号。")
        } else {
            ("managed", "当前 Grok 登录状态已匹配已保存账户。")
        }
    } else if current.is_some() || has_current_state(state)? || !enabled_ids.is_empty() {
        ("unmanaged", "当前 Grok 登录或中转配置尚未纳入本应用管理。")
    } else {
        ("missing", "尚未检测到 Grok 登录。")
    };
    Ok(AppStatus {
        profiles,
        detected_profile: current
            .as_ref()
            .filter(|_| oauth_active_id.is_none() && !relay_managed)
            .map(detected_profile)
            .transpose()?,
        auth_path: auth_path(state).display().to_string(),
        auth_state: AuthState {
            kind: kind.to_string(),
            message: message.to_string(),
        },
        autostart_enabled: app.autolaunch().is_enabled().unwrap_or(false),
        web_access: local_web::web_access_status(app, state)?,
    })
}

pub(crate) fn upsert_oauth_profile(
    state: &AppState,
    token: &grok_oauth::GrokTokenResponse,
    userinfo: Option<&grok_oauth::GrokUserInfo>,
    requested_alias: &str,
) -> Result<ProfileSummary, String> {
    if token.access_token.trim().is_empty() {
        return Err("Grok OAuth 未返回 access_token。".to_string());
    }
    if token
        .refresh_token
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Err("Grok OAuth 未返回 refresh_token，无法保存可续期凭据。".to_string());
    }
    let credential = build_credential(token, userinfo)?;
    let identity = credential_identity(&credential)?;
    let auth_json = serde_json::to_string_pretty(&credential).map_err(|error| error.to_string())?;
    let mut connection = open_database(state)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let existing = profile_id_for_identity(&transaction, &identity)?;
    let now = now_millis();
    let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
    let alias = alias_for(requested_alias, &identity);
    if transaction
        .execute(
            "UPDATE accounts SET account_id = ?1, email = ?2, alias = CASE WHEN ?3 = '' THEN alias ELSE ?3 END, auth_json = ?4, updated_at = ?5 WHERE id = ?6 AND product = 'grok'",
            params![identity.account_id, identity.email, requested_alias.trim(), auth_json, now, id],
        )
        .map_err(database_error)?
        == 0
    {
        transaction
            .execute(
                "INSERT INTO accounts (id, product, account_type, account_id, email, alias, plan_type, auth_json, created_at, updated_at, sort_order) VALUES (?1, 'grok', 'oauth', ?2, ?3, ?4, '', ?5, ?6, ?6, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'grok'), 0))",
                params![id, identity.account_id, identity.email, alias, auth_json, now],
            )
            .map_err(database_error)?;
    }
    transaction.commit().map_err(database_error)?;
    grok_profile_summary(state, &connection, &id)
}

pub(crate) fn upsert_relay_profile(
    state: &AppState,
    api_key: &str,
    requested_api_base_url: &str,
    requested_alias: &str,
) -> Result<ProfileSummary, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("API Key 不能为空。".to_string());
    }
    let api_base_url = normalize_api_base_url(requested_api_base_url)?;
    let fingerprint = credential_fingerprint(api_key);
    let auth_json = serde_json::to_string_pretty(&json!({ "key": api_key }))
        .map_err(|error| error.to_string())?;
    let connection = open_database(state)?;
    let existing = connection
        .query_row(
            "SELECT id, alias FROM accounts WHERE product = 'grok' AND account_type = 'relay' AND api_base_url = ?1 AND account_id = ?2 LIMIT 1",
            params![api_base_url, fingerprint],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    if let Some((id, _)) = existing {
        return grok_profile_summary(state, &connection, &id);
    }
    let now = now_millis();
    let id = Uuid::new_v4().to_string();
    let alias = if requested_alias.trim().is_empty() {
        relay_alias("", &api_base_url)
    } else {
        requested_alias.trim().to_string()
    };
    connection
        .execute(
            "INSERT INTO accounts (id, product, account_type, api_base_url, account_id, email, alias, plan_type, auth_json, created_at, updated_at, last_used_at, sort_order) VALUES (?1, 'grok', 'relay', ?2, ?3, '', ?4, '', ?5, ?6, ?6, NULL, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'grok'), 0))",
            params![id, api_base_url, fingerprint, alias, auth_json, now],
        )
        .map_err(database_error)?;
    grok_profile_summary(state, &connection, &id)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_relay_profile(
    state: &AppState,
    profile_id: &str,
    requested_alias: &str,
    requested_api_key: Option<&str>,
    requested_api_base_url: &str,
    model_profile_id: Option<&str>,
    default_model_id: Option<&str>,
    force: bool,
) -> Result<ProfileSummary, String> {
    let _guard = lock_configuration()?;
    models::validate_model_selection(
        state,
        AccountProduct::Grok,
        model_profile_id,
        default_model_id,
    )?;
    let mut connection = open_database(state)?;
    let mut enabled_ids = match enabled_relay_profile_ids(state, &connection) {
        Ok(ids) => ids,
        Err(_) if force => Vec::new(),
        Err(error) => return Err(error),
    };
    let was_enabled = enabled_ids.iter().any(|id| id == profile_id);
    if was_enabled {
        ensure_current_state_can_be_replaced(state, &connection, &enabled_ids, force)?;
    }
    if model_profile_id.is_none() {
        enabled_ids.retain(|id| id != profile_id);
    }
    let (account_type, auth_json) = connection
        .query_row(
            "SELECT account_type, auth_json FROM accounts WHERE id = ?1 AND product = 'grok'",
            params![profile_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type != ACCOUNT_TYPE_RELAY {
        return Err("该账户不是中转站账户。".to_string());
    }
    let previous: Value =
        serde_json::from_str(&auth_json).map_err(|_| "存档的中转站凭据已损坏。".to_string())?;
    let api_key = requested_api_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| previous.get("key").and_then(Value::as_str))
        .ok_or_else(|| "中转站账户缺少 API Key。".to_string())?;
    let api_base_url = normalize_api_base_url(requested_api_base_url)?;
    let fingerprint = credential_fingerprint(api_key);
    let duplicate = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE product = 'grok' AND account_type = 'relay' AND id <> ?1 AND api_base_url = ?2 AND account_id = ?3)",
            params![profile_id, api_base_url, fingerprint],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if duplicate {
        return Err("该中转站账号已存在。".to_string());
    }
    let alias = relay_alias(requested_alias, &api_base_url);
    let auth_json = serde_json::to_string_pretty(&json!({ "key": api_key }))
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    transaction
        .execute(
            "UPDATE accounts SET alias = ?1, api_base_url = ?2, account_id = ?3, auth_json = ?4, model_profile_id = ?5, default_model_id = ?6, updated_at = ?7 WHERE id = ?8 AND product = 'grok' AND account_type = 'relay'",
            params![
                alias,
                api_base_url,
                fingerprint,
                auth_json,
                model_profile_id,
                default_model_id,
                now_millis(),
                profile_id
            ],
        )
        .map_err(database_error)?;
    let backup = if was_enabled {
        Some(rebuild_enabled_configuration(
            state,
            &transaction,
            &enabled_ids,
        )?)
    } else {
        None
    };
    let database_result = set_enabled_relay_profile_ids(&transaction, &enabled_ids)
        .and_then(|_| transaction.commit().map_err(database_error));
    if let Err(error) = database_result {
        if let Some(backup) = backup.as_ref() {
            restore_configuration(state, backup)?;
        }
        return Err(error);
    }
    grok_profile_summary(state, &connection, profile_id)
}

pub(crate) fn import_current_profile(
    state: &AppState,
    requested_alias: Option<String>,
) -> Result<ProfileSummary, String> {
    let current =
        current_credential(state)?.ok_or_else(|| "未找到受支持的 Grok 凭据。".to_string())?;
    let credential = match current {
        CurrentCredential::OAuth(credential) => credential,
        CurrentCredential::Relay => {
            return Err("当前 Grok 中转配置不包含可导入凭据，请重新添加中转站账号。".to_string())
        }
    };
    let identity = credential_identity(&credential)?;
    let auth_json = serde_json::to_string_pretty(&credential).map_err(|error| error.to_string())?;
    let mut connection = open_database(state)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let existing = profile_id_for_identity(&transaction, &identity)?;
    let now = now_millis();
    let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
    let requested_alias = requested_alias.unwrap_or_default();
    if transaction
        .execute(
            "UPDATE accounts SET account_id = ?1, email = ?2, alias = CASE WHEN ?3 = '' THEN alias ELSE ?3 END, auth_json = ?4, updated_at = ?5 WHERE id = ?6 AND product = 'grok'",
            params![identity.account_id, identity.email, requested_alias.trim(), auth_json, now, id],
        )
        .map_err(database_error)?
        == 0
    {
        transaction
            .execute(
                "INSERT INTO accounts (id, product, account_type, account_id, email, alias, plan_type, auth_json, created_at, updated_at, sort_order) VALUES (?1, 'grok', 'oauth', ?2, ?3, ?4, '', ?5, ?6, ?6, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'grok'), 0))",
                params![id, identity.account_id, identity.email, alias_for(&requested_alias, &identity), auth_json, now],
            )
            .map_err(database_error)?;
    }
    transaction.commit().map_err(database_error)?;
    get_profile_summary_for_product(&connection, AccountProduct::Grok, &id, Some(&id))
}

pub(crate) fn switch_profile(
    state: &AppState,
    profile_id: &str,
    force: bool,
) -> Result<ProfileSummary, String> {
    let _guard = lock_configuration()?;
    let mut connection = open_database(state)?;
    let account_type = connection
        .query_row(
            "SELECT account_type FROM accounts WHERE id = ?1 AND product = 'grok'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type == ACCOUNT_TYPE_RELAY {
        set_relay_enabled_locked(state, &mut connection, profile_id, true, force)?;
        return grok_profile_summary(state, &connection, profile_id);
    }
    if account_type != ACCOUNT_TYPE_OAUTH {
        return Err("不支持的 Grok 账户类型。".to_string());
    }
    let enabled_ids = enabled_relay_profile_ids(state, &connection)?;
    ensure_current_state_can_be_replaced(state, &connection, &enabled_ids, force)?;
    let auth_json = connection
        .query_row(
            "SELECT auth_json FROM accounts WHERE id = ?1 AND product = 'grok' AND account_type = 'oauth'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(database_error)?;
    let mut credential: Value =
        serde_json::from_str(&auth_json).map_err(|_| "存档的 Grok 凭据已损坏。".to_string())?;
    refresh_credential_if_needed(&mut credential)?;
    let backup = rebuild_oauth_configuration(state, &connection, &credential)?;
    let auth_json = serde_json::to_string_pretty(&credential).map_err(|error| error.to_string())?;
    let now = now_millis();
    let database_result = (|| {
        let transaction = connection.transaction().map_err(database_error)?;
        transaction
            .execute(
                "UPDATE accounts SET auth_json = ?1, last_used_at = ?2, updated_at = ?2 WHERE id = ?3 AND product = 'grok'",
                params![auth_json, now, profile_id],
            )
            .map_err(database_error)?;
        set_enabled_relay_profile_ids(&transaction, &[])?;
        transaction.commit().map_err(database_error)
    })();
    if let Err(error) = database_result {
        restore_configuration(state, &backup)?;
        return Err(error);
    }
    grok_profile_summary(state, &connection, profile_id)
}

pub(crate) fn set_relay_enabled(
    state: &AppState,
    profile_id: &str,
    enabled: bool,
    force: bool,
) -> Result<(), String> {
    let _guard = lock_configuration()?;
    let mut connection = open_database(state)?;
    set_relay_enabled_locked(state, &mut connection, profile_id, enabled, force)
}

fn set_relay_enabled_locked(
    state: &AppState,
    connection: &mut Connection,
    profile_id: &str,
    enabled: bool,
    force: bool,
) -> Result<(), String> {
    let account_type = connection
        .query_row(
            "SELECT account_type FROM accounts WHERE id = ?1 AND product = 'grok'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type != ACCOUNT_TYPE_RELAY {
        return Err("只有 Grok 中转账号可以启用或停用。".to_string());
    }
    let mut enabled_ids = match enabled_relay_profile_ids(state, connection) {
        Ok(ids) => ids,
        Err(_) if force => Vec::new(),
        Err(error) => return Err(error),
    };
    ensure_current_state_can_be_replaced(state, connection, &enabled_ids, force)?;
    if enabled_ids.iter().any(|id| id == profile_id) == enabled {
        return Ok(());
    }
    if enabled {
        enabled_ids.push(profile_id.to_string());
    } else {
        enabled_ids.retain(|id| id != profile_id);
    }
    let backup = rebuild_enabled_configuration(state, connection, &enabled_ids)?;
    let database_result = (|| {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        set_enabled_relay_profile_ids(&transaction, &enabled_ids)?;
        if enabled {
            transaction
                .execute(
                    "UPDATE accounts SET last_used_at = ?1, updated_at = ?1 WHERE id = ?2 AND product = 'grok'",
                    params![now_millis(), profile_id],
                )
                .map_err(database_error)?;
        }
        transaction.commit().map_err(database_error)
    })();
    if let Err(error) = database_result {
        restore_configuration(state, &backup)?;
        return Err(error);
    }
    Ok(())
}

pub(crate) fn profile_auth_json(state: &AppState, profile_id: &str) -> Result<String, String> {
    let connection = open_database(state)?;
    let account_type = connection
        .query_row(
            "SELECT account_type FROM accounts WHERE id = ?1 AND product = 'grok'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type != ACCOUNT_TYPE_OAUTH {
        return Err("中转站账户不提供原始 auth.json 编辑。".to_string());
    }
    fs::read_to_string(auth_path(state))
        .map_err(|error| format!("无法读取 Grok auth.json：{error}"))
}

pub(crate) fn update_profile(
    state: &AppState,
    profile_id: &str,
    requested_alias: &str,
    auth_json: &str,
) -> Result<ProfileSummary, String> {
    if auth_json.len() > MAX_IMPORTED_AUTH_JSON_BYTES {
        return Err("auth.json 文件过大。".to_string());
    }
    let store: Value =
        serde_json::from_str(auth_json).map_err(|_| "auth.json 不是有效的 JSON。".to_string())?;
    let store = store
        .as_object()
        .cloned()
        .ok_or_else(|| "auth.json 必须是一个 JSON 对象。".to_string())?;
    let credential = store
        .get(grok_oauth::AUTH_REGISTRY_KEY)
        .cloned()
        .ok_or_else(|| "auth.json 缺少 Grok OAuth 凭据。".to_string())?;
    if !credential.is_object() {
        return Err("auth.json 必须是一个 JSON 对象。".to_string());
    }
    let identity = credential_identity(&credential)?;
    let alias = requested_alias.trim();
    if alias.is_empty() {
        return Err("别名不能为空。".to_string());
    }
    let formatted_auth_json =
        serde_json::to_string_pretty(&credential).map_err(|error| error.to_string())?;
    let mut connection = open_database(state)?;
    let account_type = connection
        .query_row(
            "SELECT account_type FROM accounts WHERE id = ?1 AND product = 'grok'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type != ACCOUNT_TYPE_OAUTH {
        return Err("中转站账户请使用中转站编辑表单。".to_string());
    }
    let active_id = active_profile_id(state, &connection)?;
    let active = active_id.as_deref() == Some(profile_id);
    if profile_id_for_identity(&connection, &identity)?.is_some_and(|id| id != profile_id) {
        return Err("该 Grok 账号已存在。".to_string());
    }
    let previous_store = active.then(|| read_store(&auth_path(state))).transpose()?;
    if active {
        write_store(&auth_path(state), &store)?;
    }
    let result = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)
        .and_then(|transaction| {
            transaction
                .execute(
                    "UPDATE accounts SET account_id = ?1, email = ?2, alias = ?3, auth_json = ?4, updated_at = ?5 WHERE id = ?6 AND product = 'grok' AND account_type = 'oauth'",
                    params![identity.account_id, identity.email, alias, formatted_auth_json, now_millis(), profile_id],
                )
                .map_err(database_error)?;
            transaction.commit().map_err(database_error)
        });
    if let Err(error) = result {
        if let Some(previous_store) = previous_store.as_ref() {
            write_store(&auth_path(state), previous_store)?;
        }
        return Err(error);
    }
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Grok,
        profile_id,
        active_id.as_deref(),
    )
}

pub(crate) fn refresh_profile_usage(
    state: &AppState,
    profile_id: &str,
) -> Result<ProfileSummary, String> {
    let connection = open_database(state)?;
    let (account_type, auth_json) = connection
        .query_row(
            "SELECT account_type, auth_json FROM accounts WHERE id = ?1 AND product = 'grok'",
            params![profile_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type == ACCOUNT_TYPE_RELAY {
        return Err("中转站账户不支持官方额度查询。".to_string());
    }
    let mut credential: Value =
        serde_json::from_str(&auth_json).map_err(|_| "存档的 Grok 凭据已损坏。".to_string())?;
    let previous_credential = credential.clone();
    refresh_credential_if_needed(&mut credential)?;
    let active_id = active_profile_id(state, &connection)?;
    if credential != previous_credential {
        if active_id.as_deref() == Some(profile_id) {
            write_managed_credential(&auth_path(state), &credential)?;
        }
        let auth_json =
            serde_json::to_string_pretty(&credential).map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE accounts SET auth_json = ?1, updated_at = ?2 WHERE id = ?3 AND product = 'grok'",
                params![auth_json, now_millis(), profile_id],
            )
            .map_err(database_error)?;
    }

    let usage = fetch_billing_usage(&credential)?;
    let now = now_millis();
    connection
        .execute(
            "UPDATE accounts SET plan_type = CASE WHEN ?1 = '' THEN plan_type ELSE ?1 END, usage_primary_percent = ?2, usage_primary_window_minutes = ?3, usage_primary_resets_at = ?4, usage_secondary_percent = NULL, usage_secondary_window_minutes = NULL, usage_secondary_resets_at = NULL, usage_updated_at = ?5 WHERE id = ?6 AND product = 'grok'",
            params![
                usage.plan_type,
                usage.primary.as_ref().map(|window| window.used_percent),
                usage.primary.as_ref().and_then(|window| window.window_minutes),
                usage.primary.as_ref().and_then(|window| window.resets_at),
                now,
                profile_id,
            ],
        )
        .map_err(database_error)?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Grok,
        profile_id,
        active_id.as_deref(),
    )
}

fn fetch_billing_usage(credential: &Value) -> Result<AccountUsage, String> {
    let access_token = credential
        .get("key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Grok 凭据缺少 access_token，请重新授权。".to_string())?;
    let user_id = credential
        .get("user_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Grok 凭据缺少 user_id，请重新授权。".to_string())?;
    let response = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?
        .get(BILLING_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("X-XAI-Token-Auth", "xai-grok-cli")
        .header("x-userid", user_id)
        .header("x-grok-client-version", env!("CARGO_PKG_VERSION"))
        .send()
        .map_err(|error| format!("Grok 额度查询失败：{error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("无法读取 Grok 额度信息：{error}"))?;
    if !status.is_success() {
        let detail = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        return Err(format!("Grok 额度查询失败：{detail}"));
    }
    parse_billing_usage(&body)
}

fn parse_billing_usage(body: &str) -> Result<AccountUsage, String> {
    let payload: BillingResponse = serde_json::from_str(body)
        .map_err(|error| format!("Grok 额度响应格式不符合预期：{error}"))?;
    let primary = payload.config.as_ref().and_then(billing_window);
    Ok(AccountUsage {
        plan_type: payload
            .subscription_tier
            .unwrap_or_default()
            .trim()
            .to_string(),
        primary,
        secondary: None,
    })
}

fn billing_window(config: &BillingConfig) -> Option<UsageWindow> {
    let used_percent = config.credit_usage_percent?;
    if !used_percent.is_finite() {
        return None;
    }
    let start = config
        .current_period
        .as_ref()
        .and_then(|period| period.start.as_deref())
        .and_then(parse_billing_time);
    let end = config
        .current_period
        .as_ref()
        .and_then(|period| period.end.as_deref())
        .and_then(parse_billing_time);
    Some(UsageWindow {
        used_percent: used_percent.clamp(0.0, 100.0),
        window_minutes: start
            .zip(end)
            .map(|(start, end)| (end - start) / 60_000)
            .filter(|minutes| *minutes > 0),
        resets_at: end,
    })
}

fn parse_billing_time(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.timestamp_millis())
}

fn auth_path(state: &AppState) -> PathBuf {
    grok_home(state, configured_grok_home()).join("auth.json")
}

#[cfg(not(test))]
fn configured_grok_home() -> Option<std::ffi::OsString> {
    std::env::var_os("GROK_HOME")
}

#[cfg(test)]
fn configured_grok_home() -> Option<std::ffi::OsString> {
    None
}

fn grok_home(state: &AppState, configured_home: Option<std::ffi::OsString>) -> PathBuf {
    configured_home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            state
                .default_codex_home
                .parent()
                .unwrap_or(&state.default_codex_home)
                .join(".grok")
        })
}

fn grok_config_path(state: &AppState) -> PathBuf {
    grok_home(state, configured_grok_home()).join("config.toml")
}

pub(crate) fn get_grok_config(state: State<'_, AppState>) -> Result<ConfigFile, String> {
    read_grok_config(&grok_config_path(&state))
}

fn read_grok_config(path: &Path) -> Result<ConfigFile, String> {
    config::read_config(path, "", "Grok config.toml")
}

pub(crate) fn validate_grok_config(content: String) -> Vec<ConfigDiagnostic> {
    config::validate_toml(&content)
}

pub(crate) fn format_grok_config(content: String) -> Result<String, String> {
    config::format_toml(&content, "config.toml")
}

pub(crate) fn save_grok_config(state: State<'_, AppState>, content: String) -> Result<(), String> {
    save_grok_config_at(&grok_config_path(&state), &content)
}

fn save_grok_config_at(path: &Path, content: &str) -> Result<(), String> {
    config::parse_toml(content, "config.toml")?;
    write_file_atomically(path, content)
}

fn read_store(path: &Path) -> Result<serde_json::Map<String, Value>, String> {
    if !path.exists() {
        return Ok(serde_json::Map::new());
    }
    let content =
        fs::read_to_string(path).map_err(|error| format!("无法读取 Grok auth.json：{error}"))?;
    if content.trim().is_empty() {
        return Ok(serde_json::Map::new());
    }
    serde_json::from_str::<Value>(&content)
        .map_err(|_| "Grok auth.json 不是有效的 JSON。".to_string())?
        .as_object()
        .cloned()
        .ok_or_else(|| "Grok auth.json 必须是一个 JSON 对象。".to_string())
}

#[cfg(test)]
fn read_managed_credential(path: &Path) -> Result<Option<Value>, String> {
    Ok(read_store(path)?
        .get(grok_oauth::AUTH_REGISTRY_KEY)
        .cloned())
}

fn has_current_state(state: &AppState) -> Result<bool, String> {
    let content = read_grok_config(&grok_config_path(state))?.content;
    if !content.trim().is_empty() {
        let document = content
            .parse::<DocumentMut>()
            .map_err(|error| format!("Grok config.toml 格式错误：{error}"))?;
        if document
            .get("endpoints")
            .and_then(|item| item.get("models_base_url"))
            .is_some()
            || document
                .get("auth")
                .and_then(|item| item.get("preferred_method"))
                .and_then(|item| item.as_str())
                == Some("api_key")
        {
            return Ok(true);
        }
    }
    Ok(!read_store(&auth_path(state))?.is_empty())
}

fn write_managed_credential(path: &Path, credential: &Value) -> Result<(), String> {
    let mut store = read_store(path)?;
    store.insert(
        grok_oauth::AUTH_REGISTRY_KEY.to_string(),
        credential.clone(),
    );
    write_store(path, &store)
}

fn write_store(path: &Path, store: &serde_json::Map<String, Value>) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "无法定位 GROK_HOME。".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建 GROK_HOME：{error}"))?;
    let lock_path = path.with_file_name("auth.json.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|error| format!("无法打开 Grok 凭据锁：{error}"))?;
    lock.lock()
        .map_err(|error| format!("无法锁定 Grok 凭据：{error}"))?;
    let content = serde_json::to_string_pretty(&store).map_err(|error| error.to_string())?;
    write_file_atomically(path, &content)
}

fn remove_auth_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("无法移除 Grok auth.json：{error}")),
    }
}

fn current_credential(state: &AppState) -> Result<Option<CurrentCredential>, String> {
    let content = read_grok_config(&grok_config_path(state))?.content;
    let document = if content.trim().is_empty() {
        DocumentMut::new()
    } else {
        content
            .parse::<DocumentMut>()
            .map_err(|error| format!("Grok config.toml 格式错误：{error}"))?
    };
    let api_base_url = document
        .get("endpoints")
        .and_then(|item| item.get("models_base_url"))
        .and_then(|item| item.as_str());
    let api_key_mode = document
        .get("auth")
        .and_then(|item| item.get("preferred_method"))
        .and_then(|item| item.as_str())
        == Some("api_key")
        || api_base_url.is_some();
    if api_key_mode {
        return Ok(Some(CurrentCredential::Relay));
    }
    Ok(read_store(&auth_path(state))?
        .get(grok_oauth::AUTH_REGISTRY_KEY)
        .cloned()
        .map(CurrentCredential::OAuth))
}

pub(crate) struct GrokFilesSnapshot {
    config: Option<String>,
    auth: Option<String>,
}

pub(crate) fn rebuild_enabled_configuration(
    state: &AppState,
    connection: &Connection,
    enabled_account_ids: &[String],
) -> Result<GrokFilesSnapshot, String> {
    rebuild_configuration(state, connection, enabled_account_ids, None)
}

fn rebuild_oauth_configuration(
    state: &AppState,
    connection: &Connection,
    credential: &Value,
) -> Result<GrokFilesSnapshot, String> {
    rebuild_configuration(state, connection, &[], Some(credential))
}

fn rebuild_configuration(
    state: &AppState,
    connection: &Connection,
    enabled_account_ids: &[String],
    oauth_credential: Option<&Value>,
) -> Result<GrokFilesSnapshot, String> {
    let config_path = grok_config_path(state);
    let snapshot = GrokFilesSnapshot {
        config: read_optional_file(&config_path, "Grok config.toml")?,
        auth: read_optional_file(&auth_path(state), "Grok auth.json")?,
    };
    let original_config = snapshot.config.as_deref().unwrap_or_default();
    let mut document = if original_config.trim().is_empty() {
        DocumentMut::new()
    } else {
        original_config
            .parse::<DocumentMut>()
            .map_err(|error| format!("Grok config.toml 格式错误：{error}"))?
    };
    let remove_endpoints = document
        .get_mut("endpoints")
        .and_then(|item| item.as_table_mut())
        .is_some_and(|table| {
            table.remove("models_base_url");
            table.is_empty()
        });
    if remove_endpoints {
        document.remove("endpoints");
    }
    models::apply_grok_model_config(connection, &mut document, enabled_account_ids)?;
    if enabled_account_ids.is_empty() {
        if document
            .get("auth")
            .and_then(|item| item.get("preferred_method"))
            .and_then(|item| item.as_str())
            == Some("api_key")
        {
            document["auth"]
                .as_table_mut()
                .map(|table| table.remove("preferred_method"));
        }
    } else {
        if document.get("auth").is_none() {
            document["auth"] = toml_table();
        }
        document["auth"]["preferred_method"] = toml_value("api_key");
    }
    let next_config = document.to_string();
    let next_store = if let Some(credential) = oauth_credential {
        let mut store = read_store(&auth_path(state))?;
        store.insert(
            grok_oauth::AUTH_REGISTRY_KEY.to_string(),
            credential.clone(),
        );
        Some(store)
    } else {
        None
    };

    save_grok_config_at(&config_path, &next_config)?;
    let auth_result = match next_store.as_ref() {
        Some(store) => write_store(&auth_path(state), store),
        None => remove_auth_file(&auth_path(state)),
    };
    if let Err(error) = auth_result {
        let restore_error = restore_configuration(state, &snapshot).err();
        return Err(restore_error
            .map(|restore_error| format!("{error}；恢复 Grok 配置失败：{restore_error}"))
            .unwrap_or(error));
    }
    Ok(snapshot)
}

pub(crate) fn restore_configuration(
    state: &AppState,
    snapshot: &GrokFilesSnapshot,
) -> Result<(), String> {
    restore_optional_file(
        &grok_config_path(state),
        snapshot.config.as_deref(),
        "Grok config.toml",
    )?;
    restore_optional_file(
        &auth_path(state),
        snapshot.auth.as_deref(),
        "Grok auth.json",
    )
}

fn read_optional_file(path: &Path, name: &str) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("无法读取 {name}：{error}")),
    }
}

fn restore_optional_file(path: &Path, content: Option<&str>, name: &str) -> Result<(), String> {
    match content {
        Some(content) => write_file_atomically(path, content),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("无法恢复 {name}：{error}")),
        },
    }
}

fn build_credential(
    token: &grok_oauth::GrokTokenResponse,
    userinfo: Option<&grok_oauth::GrokUserInfo>,
) -> Result<Value, String> {
    let access_claims = decode_claims(&token.access_token);
    let id_claims = token.id_token.as_deref().and_then(decode_claims);
    let value = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| access_claims.as_ref()?.get(*key).and_then(Value::as_str))
            .or_else(|| {
                keys.iter()
                    .find_map(|key| id_claims.as_ref()?.get(*key).and_then(Value::as_str))
            })
            .map(str::to_string)
    };
    let principal_type = userinfo
        .and_then(|user| user.principal_type.clone())
        .or_else(|| value(&["principal_type", "principalType"]));
    let principal_id = userinfo
        .and_then(|user| user.principal_id.clone())
        .or_else(|| value(&["principal_id", "principalId"]));
    let user_id = if principal_type.as_deref() == Some("Team") {
        principal_id.clone().unwrap_or_default()
    } else {
        userinfo
            .and_then(|user| user.sub.clone())
            .or_else(|| value(&["sub", "user_id"]))
            .unwrap_or_default()
    };
    let now = Utc::now();
    let mut credential = json!({
        "key": token.access_token,
        "auth_mode": "oidc",
        "create_time": now.to_rfc3339_opts(SecondsFormat::Secs, true),
        "user_id": user_id,
        "coding_data_retention_opt_out": true,
        "refresh_token": token.refresh_token,
        "oidc_issuer": grok_oauth::OIDC_ISSUER,
        "oidc_client_id": grok_oauth::OIDC_CLIENT_ID,
    });
    let object = credential.as_object_mut().unwrap();
    insert_optional(
        object,
        "email",
        userinfo
            .and_then(|user| user.email.clone())
            .or_else(|| value(&["email"])),
    );
    insert_optional(
        object,
        "first_name",
        userinfo
            .and_then(|user| user.given_name.clone())
            .or_else(|| userinfo.and_then(|user| user.name.clone()))
            .or_else(|| value(&["given_name", "first_name"])),
    );
    insert_optional(
        object,
        "last_name",
        userinfo
            .and_then(|user| user.family_name.clone())
            .or_else(|| value(&["family_name", "last_name"])),
    );
    insert_optional(object, "principal_type", principal_type);
    insert_optional(object, "principal_id", principal_id.clone());
    insert_optional(
        object,
        "team_id",
        userinfo
            .and_then(|user| user.team_id.clone())
            .or_else(|| value(&["team_id"]))
            .or_else(|| {
                (object.get("principal_type").and_then(Value::as_str) == Some("Team"))
                    .then_some(principal_id)
                    .flatten()
            }),
    );
    insert_optional(
        object,
        "team_name",
        userinfo
            .and_then(|user| user.team_name.clone())
            .or_else(|| value(&["team_name", "teamName"])),
    );
    insert_optional(object, "organization_id", value(&["organization_id"]));
    if let Some(expires_in) = token.expires_in {
        object.insert(
            "expires_at".to_string(),
            Value::String(
                (now + chrono::Duration::seconds(expires_in))
                    .to_rfc3339_opts(SecondsFormat::Secs, true),
            ),
        );
    }
    credential_identity(&credential)?;
    Ok(credential)
}

fn insert_optional(object: &mut serde_json::Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        object.insert(key.to_string(), Value::String(value));
    }
}

fn decode_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&decoded).ok()
}

struct GrokIdentity {
    account_id: String,
    email: String,
    name: String,
    team_name: String,
}

fn credential_identity(credential: &Value) -> Result<GrokIdentity, String> {
    let string = |key: &str| {
        credential
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_default()
            .to_string()
    };
    let email = string("email");
    let account_id = [string("principal_id"), string("user_id"), email.clone()]
        .into_iter()
        .find(|value| !value.is_empty())
        .ok_or_else(|| "Grok OAuth 凭据缺少可识别的账号信息。".to_string())?;
    let name = [string("first_name"), string("last_name")]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(GrokIdentity {
        account_id,
        email,
        name,
        team_name: string("team_name"),
    })
}

fn alias_for(requested_alias: &str, identity: &GrokIdentity) -> String {
    [
        requested_alias,
        &identity.team_name,
        &identity.name,
        &identity.email,
    ]
    .into_iter()
    .map(str::trim)
    .find(|value| !value.is_empty())
    .unwrap_or("Grok 账户")
    .to_string()
}

fn profile_id_for_identity(
    connection: &Connection,
    identity: &GrokIdentity,
) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT id FROM accounts WHERE product = 'grok' AND account_type = 'oauth' AND (account_id = ?1 OR (?2 <> '' AND email = ?2 COLLATE NOCASE)) LIMIT 1",
            params![identity.account_id, identity.email],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)
}

fn profile_id_for_credential(
    connection: &Connection,
    credential: &Value,
) -> Result<Option<String>, String> {
    profile_id_for_identity(connection, &credential_identity(credential)?)
}

fn active_profile_id(state: &AppState, connection: &Connection) -> Result<Option<String>, String> {
    let Some(CurrentCredential::OAuth(credential)) = current_credential(state)? else {
        return Ok(None);
    };
    profile_id_for_credential(connection, &credential)
}

pub(crate) fn lock_configuration() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    #[cfg(test)]
    return SWITCH_LOCK
        .lock()
        .map_err(|_| "Grok 账号配置锁不可用。".to_string());
    #[cfg(not(test))]
    SWITCH_LOCK
        .try_lock()
        .map_err(|_| "已有 Grok 账号配置变更正在进行，请稍后重试。".to_string())
}

pub(crate) fn enabled_relay_profile_ids(
    state: &AppState,
    connection: &Connection,
) -> Result<Vec<String>, String> {
    if let Some(value) = get_setting(connection, ENABLED_RELAY_PROFILES_SETTING)? {
        return serde_json::from_str::<Vec<String>>(&value)
            .map_err(|_| "Grok 已启用账号设置已损坏。".to_string());
    }
    let document = read_grok_document(state)?;
    match models::infer_grok_enabled_accounts(connection, &document) {
        Ok(Some(ids)) => Ok(ids),
        Ok(None) => Ok(Vec::new()),
        Err(error) => Err(format!("检测到工具外的 Grok 模型配置变更：{error}")),
    }
}

pub(crate) fn set_enabled_relay_profile_ids(
    connection: &Connection,
    profile_ids: &[String],
) -> Result<(), String> {
    let value = serde_json::to_string(profile_ids).map_err(|error| error.to_string())?;
    set_setting(connection, ENABLED_RELAY_PROFILES_SETTING, &value)
}

fn read_grok_document(state: &AppState) -> Result<DocumentMut, String> {
    let content = read_grok_config(&grok_config_path(state))?.content;
    if content.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        content
            .parse::<DocumentMut>()
            .map_err(|error| format!("Grok config.toml 格式错误：{error}"))
    }
}

fn relay_config_matches(
    state: &AppState,
    connection: &Connection,
    enabled_ids: &[String],
) -> Result<bool, String> {
    Ok(models::grok_config_matches_accounts(
        connection,
        &read_grok_document(state)?,
        enabled_ids,
    ))
}

fn ensure_current_state_can_be_replaced(
    state: &AppState,
    connection: &Connection,
    enabled_ids: &[String],
    force: bool,
) -> Result<(), String> {
    if force || !has_current_state(state)? {
        return Ok(());
    }
    let managed = match current_credential(state)? {
        Some(CurrentCredential::OAuth(credential)) => {
            profile_id_for_credential(connection, &credential)?.is_some()
        }
        Some(CurrentCredential::Relay) => {
            !enabled_ids.is_empty() && relay_config_matches(state, connection, enabled_ids)?
        }
        None => false,
    };
    if managed {
        Ok(())
    } else {
        Err("检测到工具外的 Grok 登录、API 或模型配置变更。请确认后强制覆盖。".to_string())
    }
}

fn grok_profile_summary(
    state: &AppState,
    connection: &Connection,
    profile_id: &str,
) -> Result<ProfileSummary, String> {
    let mut profile =
        get_profile_summary_for_product(connection, AccountProduct::Grok, profile_id, None)?;
    profile.is_active = if profile.account_type == ACCOUNT_TYPE_RELAY {
        enabled_relay_profile_ids(state, connection)?
            .iter()
            .any(|id| id == profile_id)
    } else {
        active_profile_id(state, connection)?.as_deref() == Some(profile_id)
    };
    Ok(profile)
}

fn detected_profile(credential: &CurrentCredential) -> Result<ProfileSummary, String> {
    let now = now_millis();
    let (account_type, api_base_url, account_id, email, alias) = match credential {
        CurrentCredential::OAuth(credential) => {
            let identity = credential_identity(credential)?;
            (
                ACCOUNT_TYPE_OAUTH.to_string(),
                None,
                identity.account_id.clone(),
                identity.email.clone(),
                alias_for("", &identity),
            )
        }
        CurrentCredential::Relay => (
            ACCOUNT_TYPE_RELAY.to_string(),
            None,
            "grok-relay".to_string(),
            String::new(),
            "Grok 中转配置".to_string(),
        ),
    };
    Ok(ProfileSummary {
        id: "detected".to_string(),
        product: AccountProduct::Grok,
        account_type,
        api_base_url,
        account_id,
        email,
        alias,
        plan_type: String::new(),
        usage_primary: None,
        usage_secondary: None,
        antigravity_quota: None,
        usage_updated_at: None,
        reset_credits_available_count: None,
        needs_reauthorization: false,
        is_renewable: true,
        is_active: true,
        last_used_at: None,
        updated_at: now,
    })
}

pub(crate) fn delete_profile(state: &AppState, profile_id: &str) -> Result<(), String> {
    let _guard = lock_configuration()?;
    let mut connection = open_database(state)?;
    let account_type = connection
        .query_row(
            "SELECT account_type FROM accounts WHERE id = ?1 AND product = 'grok'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    let mut enabled_ids = enabled_relay_profile_ids(state, &connection)?;
    let was_enabled =
        account_type == ACCOUNT_TYPE_RELAY && enabled_ids.iter().any(|id| id == profile_id);
    enabled_ids.retain(|id| id != profile_id);
    let backup = was_enabled
        .then(|| rebuild_enabled_configuration(state, &connection, &enabled_ids))
        .transpose()?;
    let database_result = (|| {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let changed = transaction
            .execute(
                "DELETE FROM accounts WHERE id = ?1 AND product = 'grok'",
                params![profile_id],
            )
            .map_err(database_error)?;
        if changed == 0 {
            return Err("账户不存在。".to_string());
        }
        set_enabled_relay_profile_ids(&transaction, &enabled_ids)?;
        transaction.commit().map_err(database_error)
    })();
    if let Err(error) = database_result {
        if let Some(backup) = backup.as_ref() {
            restore_configuration(state, backup)?;
        }
        return Err(error);
    }
    Ok(())
}

fn refresh_credential_if_needed(credential: &mut Value) -> Result<(), String> {
    let expires_at = credential
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .or_else(|| {
            credential
                .get("create_time")
                .and_then(Value::as_str)
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc) + chrono::Duration::days(30))
        });
    if expires_at.is_some_and(|expires_at| expires_at > Utc::now() + chrono::Duration::minutes(5)) {
        return Ok(());
    }
    let refresh_token = credential
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Grok 凭据已过期且缺少 refresh_token。".to_string())?;
    let token = grok_oauth::refresh_access_token(refresh_token)?;
    let object = credential
        .as_object_mut()
        .ok_or_else(|| "存档的 Grok 凭据格式不正确。".to_string())?;
    object.insert("key".to_string(), Value::String(token.access_token));
    object.insert(
        "create_time".to_string(),
        Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)),
    );
    if let Some(refresh_token) = token.refresh_token.filter(|value| !value.trim().is_empty()) {
        object.insert("refresh_token".to_string(), Value::String(refresh_token));
    }
    if let Some(expires_in) = token.expires_in {
        object.insert(
            "expires_at".to_string(),
            Value::String(
                (Utc::now() + chrono::Duration::seconds(expires_in))
                    .to_rfc3339_opts(SecondsFormat::Secs, true),
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        auth_path, current_credential, delete_profile, enabled_relay_profile_ids,
        format_grok_config, grok_config_path, grok_home, has_current_state, import_current_profile,
        parse_billing_usage, profile_auth_json, read_grok_config, read_grok_document,
        read_managed_credential, read_store, save_grok_config_at, set_relay_enabled,
        switch_profile, update_profile, upsert_relay_profile, write_managed_credential,
        CurrentCredential, ENABLED_RELAY_PROFILES_SETTING,
    };
    use crate::{
        features::{accounts::get_profile_auth_json, models},
        platform::{
            db::{get_setting, initialize_database, open_database},
            files::write_file_atomically,
            state::{AccountProduct, AppState},
        },
        products::grok::oauth as grok_oauth,
    };
    use rusqlite::params;
    use serde_json::json;
    use std::{
        fs,
        sync::{Arc, Mutex},
    };
    use toml_edit::DocumentMut;
    use uuid::Uuid;

    static TEST_SWITCH_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn reads_formats_and_safely_saves_grok_config() {
        let directory = std::env::temp_dir().join(format!("cortana-grok-{}", Uuid::new_v4()));
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        let custom_home = directory.join("custom-grok");
        assert_eq!(
            grok_home(&state, Some(custom_home.clone().into_os_string())),
            custom_home
        );
        assert_eq!(grok_home(&state, None), directory.join(".grok"));

        let path = custom_home.join("config.toml");
        assert!(read_grok_config(&path).unwrap().content.is_empty());
        let formatted =
            format_grok_config("api_key=\"secret\"\n[ui]\ntheme=\"dark\"".into()).unwrap();
        save_grok_config_at(&path, &formatted).unwrap();
        assert_eq!(read_grok_config(&path).unwrap().content, formatted);

        assert!(save_grok_config_at(&path, "api_key = [").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), formatted);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn keeps_other_scopes_when_updating_grok_oauth_credential() {
        let directory = std::env::temp_dir().join(format!("cortana-grok-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("auth.json");
        write_file_atomically(&path, r#"{"other::scope":{"key":"keep"}}"#).unwrap();
        let credential = json!({"key":"token","user_id":"u1","email":"u@example.com"});
        write_managed_credential(&path, &credential).unwrap();
        let store = read_store(&path).unwrap();
        assert_eq!(store["other::scope"]["key"], "keep");
        assert_eq!(store[grok_oauth::AUTH_REGISTRY_KEY]["key"], "token");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn reads_and_updates_grok_oauth_auth_json() {
        let _guard = TEST_SWITCH_LOCK.lock().unwrap();
        let directory = std::env::temp_dir().join(format!("cortana-grok-{}", Uuid::new_v4()));
        fs::create_dir_all(directory.join(".grok")).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let profile_id = Uuid::new_v4().to_string();
        let old_credential =
            json!({"key":"old-token","user_id":"user-1","email":"old@example.com"});
        let connection = open_database(&state).unwrap();
        connection
            .execute(
                "INSERT INTO accounts (id, product, account_type, account_id, email, alias, auth_json, created_at, updated_at) VALUES (?1, 'grok', 'oauth', 'user-1', 'old@example.com', '旧名称', ?2, 1, 1)",
                params![profile_id, old_credential.to_string()],
            )
            .unwrap();
        let original_auth = format!(
            "{{\n  \"{}\": {},\n  \"other::scope\": {{\"key\": \"original\"}}\n}}\n",
            grok_oauth::AUTH_REGISTRY_KEY,
            old_credential
        );
        write_file_atomically(&auth_path(&state), &original_auth).unwrap();

        let new_credential =
            json!({"key":"new-token","user_id":"user-1","email":"new@example.com"});
        let full_auth = json!({
            grok_oauth::AUTH_REGISTRY_KEY: new_credential,
            "other::scope": {"key": "keep"}
        });
        assert_eq!(
            profile_auth_json(&state, &profile_id).unwrap(),
            original_auth
        );
        let updated =
            update_profile(&state, &profile_id, "新名称", &full_auth.to_string()).unwrap();

        assert_eq!(updated.alias, "新名称");
        assert_eq!(updated.email, "new@example.com");
        assert_eq!(
            get_profile_auth_json(&connection, &profile_id, AccountProduct::Grok).unwrap(),
            serde_json::to_string_pretty(&full_auth[grok_oauth::AUTH_REGISTRY_KEY]).unwrap()
        );
        let saved_store = read_store(&auth_path(&state)).unwrap();
        assert_eq!(
            saved_store[grok_oauth::AUTH_REGISTRY_KEY]["key"],
            "new-token"
        );
        assert_eq!(saved_store["other::scope"]["key"], "keep");
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn detects_grok_relay_without_auth_json() {
        let directory = std::env::temp_dir().join(format!("cortana-grok-{}", Uuid::new_v4()));
        fs::create_dir_all(directory.join(".grok")).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        save_grok_config_at(
            &grok_config_path(&state),
            "[auth]\npreferred_method = \"api_key\"\n",
        )
        .unwrap();

        assert!(matches!(
            current_credential(&state).unwrap(),
            Some(CurrentCredential::Relay)
        ));
        assert!(has_current_state(&state).unwrap());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn switches_between_grok_relay_and_oauth() {
        let _guard = TEST_SWITCH_LOCK.lock().unwrap();
        let directory = std::env::temp_dir().join(format!("cortana-grok-{}", Uuid::new_v4()));
        fs::create_dir_all(directory.join(".grok")).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        save_grok_config_at(
            &grok_config_path(&state),
            "[ui]\nyolo = false\n\n[endpoints]\nmodels_base_url = \"https://old.example/v1\"\n\n[model.user]\nmodel = \"keep\"\n",
        )
        .unwrap();
        let relay =
            upsert_relay_profile(&state, "relay-key", "https://relay.example/v1", "Relay").unwrap();
        models::save_model_profile(
            &state,
            AccountProduct::Grok,
            None,
            "Relay models",
            vec![models::ModelEntry {
                id: "relay-model".to_string(),
                display_name: "Relay Model".to_string(),
                claude_slot: None,
                context_1m: false,
            }],
            vec![models::ModelAssignment {
                account_id: relay.id.clone(),
                account_alias: relay.alias.clone(),
                default_model_id: Some("relay-model".to_string()),
            }],
            false,
        )
        .unwrap();
        write_file_atomically(
            &auth_path(&state),
            r#"{"other::scope":{"key":"remove-with-file"}}"#,
        )
        .unwrap();
        switch_profile(&state, &relay.id, true).unwrap();

        assert!(!auth_path(&state).exists());
        let connection = open_database(&state).unwrap();
        assert_eq!(
            enabled_relay_profile_ids(&state, &connection).unwrap(),
            vec![relay.id.clone()]
        );
        drop(connection);
        let relay_config = read_grok_config(&grok_config_path(&state))
            .unwrap()
            .content
            .parse::<DocumentMut>()
            .unwrap();
        assert!(relay_config.get("endpoints").is_none());
        assert_eq!(
            relay_config["auth"]["preferred_method"].as_str(),
            Some("api_key")
        );
        assert!(relay_config
            .get("models")
            .and_then(|item| item.get("default"))
            .and_then(|item| item.as_str())
            .is_some_and(|key| key.starts_with(&format!("cortana-{}", &relay.id[..8]))));
        assert_eq!(
            relay_config["model"]["user"]["model"].as_str(),
            Some("keep")
        );

        let oauth_id = Uuid::new_v4().to_string();
        let oauth = json!({
            "key": "oauth-token",
            "user_id": "oauth-user",
            "email": "oauth@example.com",
            "refresh_token": "oauth-refresh",
            "expires_at": "2030-01-01T00:00:00Z"
        });
        open_database(&state)
            .unwrap()
            .execute(
                "INSERT INTO accounts (id, product, account_type, account_id, email, alias, auth_json, created_at, updated_at) VALUES (?1, 'grok', 'oauth', 'oauth-user', 'oauth@example.com', 'OAuth', ?2, 1, 1)",
                params![oauth_id, oauth.to_string()],
            )
            .unwrap();
        switch_profile(&state, &oauth_id, true).unwrap();

        let store = read_store(&auth_path(&state)).unwrap();
        assert_eq!(store[grok_oauth::AUTH_REGISTRY_KEY]["key"], "oauth-token");
        assert_eq!(
            get_setting(
                &open_database(&state).unwrap(),
                ENABLED_RELAY_PROFILES_SETTING
            )
            .unwrap()
            .as_deref(),
            Some("[]")
        );
        let oauth_config = read_grok_config(&grok_config_path(&state))
            .unwrap()
            .content
            .parse::<DocumentMut>()
            .unwrap();
        assert!(oauth_config.get("endpoints").is_none());
        assert!(oauth_config["auth"].get("preferred_method").is_none());
        assert_eq!(
            oauth_config["model"]["user"]["model"].as_str(),
            Some("keep")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn enables_disables_and_deletes_multiple_grok_relays() {
        let _guard = TEST_SWITCH_LOCK.lock().unwrap();
        let directory = std::env::temp_dir().join(format!("cortana-grok-{}", Uuid::new_v4()));
        fs::create_dir_all(directory.join(".grok")).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let first =
            upsert_relay_profile(&state, "first-key", "https://first.example/v1", "First").unwrap();
        let second =
            upsert_relay_profile(&state, "second-key", "https://second.example/v1", "Second")
                .unwrap();
        assert_eq!(
            set_relay_enabled(&state, &first.id, true, true).unwrap_err(),
            "Grok 中转账号“First”请先关联模型方案。"
        );
        let model_profile = models::save_model_profile(
            &state,
            AccountProduct::Grok,
            None,
            "Shared",
            vec![models::ModelEntry {
                id: "relay-model".to_string(),
                display_name: "Relay Model".to_string(),
                claude_slot: None,
                context_1m: false,
            }],
            vec![
                models::ModelAssignment {
                    account_id: first.id.clone(),
                    account_alias: first.alias.clone(),
                    default_model_id: Some("relay-model".to_string()),
                },
                models::ModelAssignment {
                    account_id: second.id.clone(),
                    account_alias: second.alias.clone(),
                    default_model_id: Some("relay-model".to_string()),
                },
            ],
            false,
        )
        .unwrap();

        set_relay_enabled(&state, &first.id, true, true).unwrap();
        set_relay_enabled(&state, &second.id, true, false).unwrap();
        let connection = open_database(&state).unwrap();
        assert_eq!(
            enabled_relay_profile_ids(&state, &connection).unwrap(),
            vec![first.id.clone(), second.id.clone()]
        );
        drop(connection);
        let config = read_grok_document(&state).unwrap();
        assert_eq!(
            config["model"][&format!("cortana-{}-0", &first.id[..8])]["api_key"].as_str(),
            Some("first-key")
        );
        assert_eq!(
            config["model"][&format!("cortana-{}-0", &second.id[..8])]["api_key"].as_str(),
            Some("second-key")
        );

        models::save_model_profile(
            &state,
            AccountProduct::Grok,
            Some(&model_profile.id),
            "Shared",
            vec![models::ModelEntry {
                id: "relay-model".to_string(),
                display_name: "Relay Model".to_string(),
                claude_slot: None,
                context_1m: false,
            }],
            vec![models::ModelAssignment {
                account_id: second.id.clone(),
                account_alias: second.alias.clone(),
                default_model_id: Some("relay-model".to_string()),
            }],
            false,
        )
        .unwrap();
        let connection = open_database(&state).unwrap();
        assert_eq!(
            enabled_relay_profile_ids(&state, &connection).unwrap(),
            vec![second.id.clone()]
        );
        drop(connection);
        delete_profile(&state, &second.id).unwrap();
        let config = read_grok_document(&state).unwrap();
        assert!(config
            .get("model")
            .and_then(|item| item.as_table())
            .is_none_or(|table| !table.iter().any(|(key, _)| key.starts_with("cortana-"))));
        assert!(config["auth"].get("preferred_method").is_none());
        assert_eq!(
            enabled_relay_profile_ids(&state, &open_database(&state).unwrap()).unwrap(),
            Vec::<String>::new()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn refuses_to_replace_an_unmanaged_grok_login() {
        let _guard = TEST_SWITCH_LOCK.lock().unwrap();
        let directory = std::env::temp_dir().join(format!("cortana-grok-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let path = auth_path(&state);
        let target = json!({
            "key": "target-token",
            "user_id": "target-user",
            "email": "target@example.com",
            "refresh_token": "target-refresh",
            "create_time": "2026-01-01T00:00:00Z",
            "expires_at": "2030-01-01T00:00:00Z"
        });
        write_managed_credential(&path, &target).unwrap();
        let target = import_current_profile(&state, None).unwrap();
        write_managed_credential(
            &path,
            &json!({
                "key": "external-token",
                "user_id": "external-user",
                "email": "external@example.com",
                "refresh_token": "external-refresh",
                "create_time": "2026-01-01T00:00:00Z",
                "expires_at": "2030-01-01T00:00:00Z"
            }),
        )
        .unwrap();
        let error = switch_profile(&state, &target.id, false).unwrap_err();
        assert!(error.contains("工具外"), "{error}");
        switch_profile(&state, &target.id, true).unwrap();
        assert_eq!(
            read_managed_credential(&path).unwrap().unwrap()["user_id"],
            "target-user"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn parses_grok_weekly_billing_usage() {
        let usage = parse_billing_usage(
            r#"{"config":{"creditUsagePercent":25.5,"currentPeriod":{"start":"2026-07-22T00:00:00Z","end":"2026-07-29T00:00:00Z"}},"subscriptionTier":"SuperGrok"}"#,
        )
        .unwrap();
        assert_eq!(usage.plan_type, "SuperGrok");
        let window = usage.primary.unwrap();
        assert_eq!(window.used_percent, 25.5);
        assert_eq!(window.window_minutes, Some(7 * 24 * 60));
        assert_eq!(window.resets_at, Some(1785283200000));
    }
}
