use super::oauth::{chatgpt_user_id_from_auth_json, identity_from_auth_json};
use crate::{
    features::{
        gateway::{self, UpstreamAuthMode, UpstreamProtocol, DEFAULT_ANTHROPIC_MAX_TOKENS},
        models,
    },
    platform::{
        db::{credential_fingerprint, database_error, get_setting, open_database, set_setting},
        local_web,
        state::{
            now_millis, AccountProduct, AppState, AppStatus, AuthState, Identity, ProfileSummary,
            UsageWindow, ACCOUNT_TYPE_OAUTH, ACCOUNT_TYPE_RELAY, MAX_IMPORTED_AUTH_JSON_BYTES,
        },
    },
    products::{
        claude,
        codex::{
            apply_gateway_profile_files_with_model, apply_profile_files,
            apply_profile_files_with_model, auth_path, build_relay_auth_json,
            clear_managed_profile_files, extract_api_key, extract_api_key_from_value,
            extract_refresh_token, has_usable_credential, normalize_api_base_url, read_auth_json,
            read_provider_config, restore_profile_files, usage::fetch_account_usage,
        },
    },
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use std::path::Path;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use url::Url;
use uuid::Uuid;

pub(crate) fn app_status(app: &tauri::AppHandle, state: &AppState) -> Result<AppStatus, String> {
    let connection = open_database(state)?;
    let auth_path = auth_path(state)?;
    let (auth_state, active_profile_id) = resolve_auth_state(&connection, &auth_path)?;
    let profiles = list_profiles(&connection, active_profile_id.as_deref())?;
    let detected_profile = if active_profile_id.is_some() {
        None
    } else if let Some(profile) =
        detected_profile_from_config(&auth_path.with_file_name("config.toml"))?
    {
        Some(profile)
    } else {
        read_auth_json(&auth_path)?
            .filter(|auth_json| has_usable_credential(auth_json))
            .map(|auth_json| {
                detected_profile_from_auth(
                    auth_json.as_str(),
                    &auth_path.with_file_name("config.toml"),
                )
            })
            .transpose()?
            .flatten()
    };
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    Ok(AppStatus {
        profiles,
        detected_profile,
        auth_path: auth_path.display().to_string(),
        auth_state,
        autostart_enabled,
        web_access: local_web::web_access_status(app, state)?,
    })
}

pub(crate) fn list_profiles(
    connection: &Connection,
    active_id: Option<&str>,
) -> Result<Vec<ProfileSummary>, String> {
    list_profiles_for_product(connection, AccountProduct::Codex, active_id)
}

pub(crate) fn list_profiles_for_product(
    connection: &Connection,
    product: AccountProduct,
    active_id: Option<&str>,
) -> Result<Vec<ProfileSummary>, String> {
    let mut statement = connection
        .prepare("SELECT id, account_type, api_base_url, account_id, email, alias, plan_type, usage_primary_percent, usage_primary_window_minutes, usage_primary_resets_at, usage_secondary_percent, usage_secondary_window_minutes, usage_secondary_resets_at, usage_updated_at, last_used_at, updated_at, reset_credits_available_count, antigravity_quota_json, auth_json, oauth_invalidated_at, upstream_protocol, upstream_auth_mode, anthropic_max_tokens FROM accounts WHERE product = ?1 AND (?1 <> 'antigravity' OR account_type = 'oauth') ORDER BY sort_order ASC, created_at ASC")
        .map_err(database_error)?;
    let rows = statement
        .query_map(params![product.as_str()], |row| {
            profile_summary_from_row(row, product, active_id)
        })
        .map_err(database_error)?;
    let profiles = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(profiles)
}

pub(crate) fn profile_summary_from_row(
    row: &rusqlite::Row<'_>,
    product: AccountProduct,
    active_id: Option<&str>,
) -> rusqlite::Result<ProfileSummary> {
    let id: String = row.get(0)?;
    let primary_percent: Option<f64> = row.get(7)?;
    let primary_window_minutes: Option<i64> = row.get(8)?;
    let primary_resets_at: Option<i64> = row.get(9)?;
    let secondary_percent: Option<f64> = row.get(10)?;
    let secondary_window_minutes: Option<i64> = row.get(11)?;
    let secondary_resets_at: Option<i64> = row.get(12)?;
    let antigravity_quota = row
        .get::<_, Option<String>>(17)?
        .and_then(|value| serde_json::from_str(&value).ok());
    let is_renewable = if product == AccountProduct::Claude {
        row.get::<_, String>(18)
            .ok()
            .is_some_and(|auth_json| claude::credential_is_renewable(&auth_json))
    } else {
        true
    };
    Ok(ProfileSummary {
        is_active: active_id == Some(id.as_str()),
        id,
        product,
        account_type: row.get(1)?,
        api_base_url: row.get(2)?,
        upstream_protocol: row.get(20)?,
        upstream_auth_mode: row.get(21)?,
        anthropic_max_tokens: row.get(22)?,
        account_id: row.get(3)?,
        email: row.get(4)?,
        alias: row.get(5)?,
        plan_type: row.get(6)?,
        usage_primary: primary_percent.map(|used_percent| UsageWindow {
            used_percent,
            window_minutes: primary_window_minutes,
            resets_at: primary_resets_at,
        }),
        usage_secondary: secondary_percent.map(|used_percent| UsageWindow {
            used_percent,
            window_minutes: secondary_window_minutes,
            resets_at: secondary_resets_at,
        }),
        antigravity_quota,
        usage_updated_at: row.get(13)?,
        reset_credits_available_count: row.get(16)?,
        needs_reauthorization: product == AccountProduct::Codex
            && row.get::<_, Option<i64>>(19)?.is_some(),
        is_renewable,
        last_used_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

pub(crate) fn get_profile_summary(
    connection: &Connection,
    profile_id: &str,
    active_id: Option<&str>,
) -> Result<ProfileSummary, String> {
    get_profile_summary_for_product(connection, AccountProduct::Codex, profile_id, active_id)
}

pub(crate) fn get_profile_summary_for_product(
    connection: &Connection,
    product: AccountProduct,
    profile_id: &str,
    active_id: Option<&str>,
) -> Result<ProfileSummary, String> {
    connection
        .query_row(
            "SELECT id, account_type, api_base_url, account_id, email, alias, plan_type, usage_primary_percent, usage_primary_window_minutes, usage_primary_resets_at, usage_secondary_percent, usage_secondary_window_minutes, usage_secondary_resets_at, usage_updated_at, last_used_at, updated_at, reset_credits_available_count, antigravity_quota_json, auth_json, oauth_invalidated_at, upstream_protocol, upstream_auth_mode, anthropic_max_tokens FROM accounts WHERE id = ?1 AND product = ?2 AND (?2 <> 'antigravity' OR account_type = 'oauth')",
            params![profile_id, product.as_str()],
            |row| profile_summary_from_row(row, product, active_id),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())
}

pub(crate) fn get_profile_auth_json(
    connection: &Connection,
    profile_id: &str,
    product: AccountProduct,
) -> Result<String, String> {
    let profile = connection
        .query_row(
            "SELECT account_type, auth_json FROM accounts WHERE id = ?1 AND product = ?2",
            params![profile_id, product.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if profile.0 != ACCOUNT_TYPE_OAUTH {
        return Err("中转站账户不提供原始 auth.json 编辑。".to_string());
    }
    Ok(profile.1)
}

pub(crate) fn relay_api_key_for_profile(
    connection: &Connection,
    profile_id: &str,
    product: AccountProduct,
) -> Result<String, String> {
    let (account_type, auth_json) = connection
        .query_row(
            "SELECT account_type, auth_json FROM accounts WHERE id = ?1 AND product = ?2",
            params![profile_id, product.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type != ACCOUNT_TYPE_RELAY {
        return Err("该账户不是中转站账户。".to_string());
    }
    let auth: Value =
        serde_json::from_str(&auth_json).map_err(|_| "存档的中转站凭据已损坏。".to_string())?;
    let field = match product {
        AccountProduct::Codex => "OPENAI_API_KEY",
        AccountProduct::Claude => "authToken",
        AccountProduct::Grok => "key",
        AccountProduct::Antigravity => {
            return Err("Antigravity 仅支持浏览器 OAuth 账户。".to_string());
        }
    };
    auth.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "中转站账户缺少 API Key。".to_string())
}

pub(crate) fn detected_profile_from_auth(
    auth_json: &str,
    config_path: &Path,
) -> Result<Option<ProfileSummary>, String> {
    let auth: Value =
        serde_json::from_str(auth_json).map_err(|_| "auth.json 不是有效的 JSON。".to_string())?;
    if extract_api_key_from_value(&auth).is_some() {
        let (_, provider_name, api_base_url, _) = read_provider_config(config_path)?;
        return Ok(Some(detected_relay_profile(provider_name, api_base_url)));
    }
    let identity = identity_from_auth_json(&auth);

    let now = now_millis();
    let usage = fetch_account_usage(auth_json, &identity.account_id).ok();
    let alias = oauth_alias("", &identity);
    Ok(Some(ProfileSummary {
        id: "detected".to_string(),
        product: AccountProduct::Codex,
        account_type: ACCOUNT_TYPE_OAUTH.to_string(),
        api_base_url: None,
        upstream_protocol: "openaiResponses".to_string(),
        upstream_auth_mode: "bearer".to_string(),
        anthropic_max_tokens: 16_384,
        account_id: identity.account_id,
        email: identity.email,
        alias,
        plan_type: usage
            .as_ref()
            .filter(|usage| !usage.plan_type.is_empty())
            .map(|usage| usage.plan_type.clone())
            .unwrap_or(identity.plan_type),
        usage_primary: usage.as_ref().and_then(|usage| usage.primary.clone()),
        usage_secondary: usage.as_ref().and_then(|usage| usage.secondary.clone()),
        antigravity_quota: None,
        usage_updated_at: usage.map(|_| now),
        reset_credits_available_count: None,
        needs_reauthorization: false,
        is_renewable: true,
        is_active: true,
        last_used_at: None,
        updated_at: now,
    }))
}

fn detected_profile_from_config(config_path: &Path) -> Result<Option<ProfileSummary>, String> {
    let (_, provider_name, api_base_url, bearer_token) = read_provider_config(config_path)?;
    Ok(bearer_token.map(|_| detected_relay_profile(provider_name, api_base_url)))
}

fn detected_relay_profile(
    provider_name: Option<String>,
    api_base_url: Option<String>,
) -> ProfileSummary {
    let alias = provider_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            api_base_url
                .as_deref()
                .and_then(|base_url| Url::parse(base_url).ok())
                .and_then(|url| url.host_str().map(str::to_string))
        })
        .unwrap_or_else(|| "当前 API Key".to_string());
    ProfileSummary {
        id: "detected".to_string(),
        product: AccountProduct::Codex,
        account_type: ACCOUNT_TYPE_RELAY.to_string(),
        api_base_url,
        upstream_protocol: "openaiResponses".to_string(),
        upstream_auth_mode: "bearer".to_string(),
        anthropic_max_tokens: 16_384,
        account_id: String::new(),
        email: String::new(),
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
        updated_at: now_millis(),
    }
}

pub(crate) fn profile_id_for_auth(
    connection: &Connection,
    auth_json: &str,
    config_path: &Path,
) -> Result<Option<String>, String> {
    if let Some(api_key) = extract_api_key(auth_json)? {
        let (_, _, api_base_url, _) = read_provider_config(config_path)?;
        return relay_profile_id_for_credentials(connection, api_base_url.as_deref(), &api_key);
    }
    let Ok(auth) = serde_json::from_str(auth_json) else {
        return Ok(None);
    };
    let identity = identity_from_auth_json(&auth);
    let user_id = chatgpt_user_id_from_auth_json(&auth);
    Ok(find_codex_oauth_profile(connection, &identity.account_id, &user_id)?.map(|(id, _, _)| id))
}

fn relay_profile_id_for_credentials(
    connection: &Connection,
    api_base_url: Option<&str>,
    api_key: &str,
) -> Result<Option<String>, String> {
    let Some(api_base_url) = api_base_url.and_then(|url| normalize_api_base_url(url).ok()) else {
        return Ok(None);
    };
    connection
        .query_row(
            "SELECT id FROM accounts WHERE product = 'codex' AND account_type = 'relay' AND api_base_url = ?1 AND account_id = ?2 AND upstream_protocol = 'openaiResponses' LIMIT 1",
            params![api_base_url, credential_fingerprint(api_key)],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)
}

pub(crate) fn find_codex_oauth_profile(
    connection: &Connection,
    account_id: &str,
    user_id: &str,
) -> Result<Option<(String, String, String)>, String> {
    if account_id.is_empty() || user_id.is_empty() {
        return Ok(None);
    }
    let profiles = connection
        .prepare(
            "SELECT id, alias, email FROM accounts WHERE product = 'codex' AND account_type = 'oauth' AND account_id = ?1 AND chatgpt_user_id = ?2 LIMIT 2",
        )
        .map_err(database_error)?
        .query_map(params![account_id, user_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    match profiles.as_slice() {
        [] => Ok(None),
        [profile] => Ok(Some(profile.clone())),
        _ => Err("检测到重复的 Codex 账号档案，请先删除重复账号。".to_string()),
    }
}

pub(crate) fn resolve_auth_state(
    connection: &Connection,
    path: &Path,
) -> Result<(AuthState, Option<String>), String> {
    let config_path = path.with_file_name("config.toml");
    let (_, _, api_base_url, bearer_token) = read_provider_config(&config_path)?;
    if api_base_url.as_deref().is_some_and(gateway::is_base_url) {
        let profile_id = if gateway::is_enabled(connection)?
            && bearer_token.as_deref().is_some_and(|token| {
                gateway::local_api_key(connection)
                    .ok()
                    .flatten()
                    .is_some_and(|key| {
                        credential_fingerprint(token) == credential_fingerprint(&key)
                    })
            }) {
            get_setting(connection, gateway::ACTIVE_PROFILE_SETTING)?.filter(|profile_id| {
                connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1 AND product = 'codex')",
                        params![profile_id],
                        |row| row.get::<_, bool>(0),
                    )
                    .unwrap_or(false)
            })
        } else {
            None
        };
        return Ok((
            AuthState {
                kind: if profile_id.is_some() {
                    "managed"
                } else {
                    "unmanaged"
                }
                .to_string(),
                message: if profile_id.is_some() {
                    "当前 Codex 网关已匹配已保存账户。"
                } else {
                    "当前 Codex 网关配置未匹配可用账户。"
                }
                .to_string(),
            },
            profile_id,
        ));
    }
    let (profile_id, auth_json) = if let Some(api_key) = bearer_token {
        (
            relay_profile_id_for_credentials(
                connection,
                api_base_url.as_deref(),
                api_key.as_str(),
            )?,
            None,
        )
    } else {
        let Some(auth_json) = read_auth_json(path)? else {
            return Ok((
                AuthState {
                    kind: "missing".to_string(),
                    message: "尚未检测到 Codex 登录凭据。".to_string(),
                },
                None,
            ));
        };
        let profile_id = profile_id_for_auth(connection, &auth_json, &config_path)?;
        (profile_id, Some(auth_json))
    };
    if let (Some(profile_id), Some(auth_json)) = (profile_id.as_deref(), auth_json.as_deref()) {
        connection
            .execute(
                "UPDATE accounts SET auth_json = ?1, oauth_invalidated_at = NULL, updated_at = ?2 WHERE id = ?3 AND product = 'codex' AND account_type = 'oauth' AND auth_json <> ?1",
                params![auth_json, now_millis(), profile_id],
            )
            .map_err(database_error)?;
    }
    let auth_state = if profile_id.is_some() {
        AuthState {
            kind: "managed".to_string(),
            message: "当前 Codex 登录状态已匹配已保存账户。".to_string(),
        }
    } else {
        AuthState {
            kind: "unmanaged".to_string(),
            message: "当前 Codex 登录状态尚未纳入本应用管理。".to_string(),
        }
    };
    Ok((auth_state, profile_id))
}

pub(crate) fn switch_profile_internal(
    state: &AppState,
    profile_id: &str,
    force: bool,
) -> Result<ProfileSummary, String> {
    let mut connection = open_database(state)?;
    let path = auth_path(state)?;
    let (_, active_id) = resolve_auth_state(&connection, &path)?;
    let external_auth_has_credential = read_provider_config(&path.with_file_name("config.toml"))?
        .3
        .is_some()
        || read_auth_json(&path)?.is_some_and(|auth_json| has_usable_credential(&auth_json));
    if active_id.is_none() && external_auth_has_credential && !force {
        return Err(
            "检测到工具外的 Codex 登录或 API 配置变更。请先导入当前状态，或确认后强制切换。"
                .to_string(),
        );
    }
    let row = connection
        .query_row(
            "SELECT id, account_type, api_base_url, auth_json, model_profile_id, default_model_id, upstream_protocol FROM accounts WHERE id = ?1 AND product = 'codex'",
            params![profile_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    let protocol = UpstreamProtocol::parse(&row.6)?;
    let gateway_enabled = gateway::is_enabled(&connection)?;
    if protocol.requires_gateway() && !gateway_enabled {
        return Err("该账号协议需要先启用网关模式。".to_string());
    }
    let backup = if gateway_enabled {
        gateway::ensure_available()?;
        let local_api_key = gateway::ensure_local_api_key(&connection)?;
        apply_gateway_profile_files_with_model(
            state,
            &local_api_key,
            row.4.as_deref(),
            row.5.as_deref(),
            protocol,
        )?
    } else {
        apply_profile_files_with_model(
            state,
            &row.3,
            &row.1,
            row.2.as_deref(),
            row.4.as_deref(),
            row.5.as_deref(),
            protocol,
        )?
    };
    let now = now_millis();
    let database_result = (|| {
        let transaction = connection.transaction().map_err(database_error)?;
        transaction
            .execute(
                "UPDATE accounts SET last_used_at = ?1, updated_at = ?1 WHERE id = ?2 AND product = 'codex'",
                params![now, row.0],
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![gateway::ACTIVE_PROFILE_SETTING, row.0],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)
    })();
    if let Err(error) = database_result {
        restore_profile_files(state, &backup)?;
        return Err(error);
    }
    get_profile_summary(&connection, &row.0, Some(&row.0))
}

pub(crate) fn set_gateway_mode_internal(
    state: &AppState,
    enabled: bool,
    profile_id: Option<&str>,
) -> Result<gateway::GatewayStatus, String> {
    let connection = open_database(state)?;
    let currently_enabled = gateway::is_enabled(&connection)?;
    let path = auth_path(state)?;
    let (_, current_profile_id) = resolve_auth_state(&connection, &path)?;
    let target_profile_id = profile_id.or(current_profile_id.as_deref());

    if enabled {
        gateway::ensure_available()?;
        gateway::ensure_local_api_key(&connection)?;
        set_setting(&connection, gateway::GATEWAY_ENABLED_SETTING, "true")?;
        if let Some(profile_id) = target_profile_id {
            if let Err(error) = switch_profile_internal(state, profile_id, true) {
                set_setting(
                    &connection,
                    gateway::GATEWAY_ENABLED_SETTING,
                    &currently_enabled.to_string(),
                )?;
                return Err(error);
            }
        }
        return gateway::gateway_status(state);
    }

    if !currently_enabled {
        return gateway::gateway_status(state);
    }
    let Some(profile_id) = target_profile_id else {
        set_setting(&connection, gateway::GATEWAY_ENABLED_SETTING, "false")?;
        return gateway::gateway_status(state);
    };
    let protocol = connection
        .query_row(
            "SELECT upstream_protocol FROM accounts WHERE id = ?1 AND product = 'codex'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .map(|value| UpstreamProtocol::parse(&value))
        .transpose()?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if protocol.requires_gateway() {
        let backup = clear_managed_profile_files(state)?;
        let result = (|| {
            set_setting(&connection, gateway::GATEWAY_ENABLED_SETTING, "false")?;
            connection
                .execute(
                    "DELETE FROM settings WHERE key = ?1",
                    params![gateway::ACTIVE_PROFILE_SETTING],
                )
                .map_err(database_error)?;
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            restore_profile_files(state, &backup)?;
            return Err(error);
        }
    } else {
        set_setting(&connection, gateway::GATEWAY_ENABLED_SETTING, "false")?;
        if let Err(error) = switch_profile_internal(state, profile_id, true) {
            set_setting(&connection, gateway::GATEWAY_ENABLED_SETTING, "true")?;
            return Err(error);
        }
    }
    gateway::gateway_status(state)
}

pub(crate) fn update_profile_internal(
    state: &AppState,
    profile_id: &str,
    requested_alias: &str,
    auth_json: &str,
) -> Result<ProfileSummary, String> {
    if auth_json.len() > MAX_IMPORTED_AUTH_JSON_BYTES {
        return Err("auth.json 文件过大。".to_string());
    }
    let parsed: Value =
        serde_json::from_str(auth_json).map_err(|_| "auth.json 不是有效的 JSON。".to_string())?;
    if !parsed.is_object() {
        return Err("auth.json 必须是一个 JSON 对象。".to_string());
    }
    let formatted_auth_json =
        serde_json::to_string_pretty(&parsed).map_err(|error| error.to_string())?;

    let mut identity = identity_from_auth_json(&parsed);
    let user_id = chatgpt_user_id_from_auth_json(&parsed);
    if identity.account_id.is_empty() || user_id.is_empty() {
        return Err("auth.json 缺少 Codex 账号或用户标识，请重新授权。".to_string());
    }
    let mut connection = open_database(state)?;
    let (existing_email, account_type) = connection
        .query_row(
            "SELECT email, account_type FROM accounts WHERE id = ?1 AND product = 'codex'",
            params![profile_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type != ACCOUNT_TYPE_OAUTH {
        return Err("中转站账户请使用中转站编辑表单。".to_string());
    }
    if identity.email.is_empty() {
        identity.email = existing_email;
    }
    let alias = oauth_alias(requested_alias, &identity);
    let auth_path = auth_path(state)?;
    let (_, active_id) = resolve_auth_state(&connection, &auth_path)?;
    let active = active_id.as_deref() == Some(profile_id);
    let backup = if !active {
        None
    } else if gateway::is_enabled(&connection)? {
        Some(apply_gateway_profile_files_with_model(
            state,
            &gateway::ensure_local_api_key(&connection)?,
            None,
            None,
            UpstreamProtocol::OpenAiResponses,
        )?)
    } else {
        Some(apply_profile_files(
            state,
            &formatted_auth_json,
            ACCOUNT_TYPE_OAUTH,
            None,
        )?)
    };
    let database_result = (|| {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        if find_codex_oauth_profile(&transaction, &identity.account_id, &user_id)?
            .is_some_and(|(id, _, _)| id != profile_id)
        {
            return Err("该 Codex 账号已存在。".to_string());
        }
        let changed = transaction
            .execute(
                "UPDATE accounts SET account_id = ?1, chatgpt_user_id = ?2, email = ?3, alias = ?4, plan_type = CASE WHEN ?5 = '' THEN plan_type ELSE ?5 END, auth_json = ?6, oauth_invalidated_at = NULL, updated_at = ?7 WHERE id = ?8 AND product = 'codex'",
                params![identity.account_id, user_id, identity.email, alias, identity.plan_type, formatted_auth_json, now_millis(), profile_id],
            )
            .map_err(database_error)?;
        if changed == 0 {
            return Err("账户不存在。".to_string());
        }
        transaction.commit().map_err(database_error)
    })();
    if let Err(error) = database_result {
        if let Some(backup) = backup.as_ref() {
            restore_profile_files(state, backup)?;
        }
        return Err(error);
    }
    let (_, active_id) = resolve_auth_state(&connection, &auth_path)?;
    get_profile_summary(&connection, profile_id, active_id.as_deref())
}

pub(crate) fn upsert_profile_from_auth(
    state: &AppState,
    auth_json: &str,
    requested_alias: &str,
) -> Result<ProfileSummary, String> {
    let parsed: Value =
        serde_json::from_str(auth_json).map_err(|_| "auth.json 不是有效的 JSON。".to_string())?;
    if !parsed.is_object() {
        return Err("auth.json 必须是一个 JSON 对象。".to_string());
    }
    extract_refresh_token(auth_json)?;
    let identity = identity_from_auth_json(&parsed);
    let user_id = chatgpt_user_id_from_auth_json(&parsed);
    if identity.account_id.is_empty() || user_id.is_empty() {
        return Err("auth.json 缺少 Codex 账号或用户标识，请重新授权。".to_string());
    }
    let mut connection = open_database(state)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let existing = find_codex_oauth_profile(&transaction, &identity.account_id, &user_id)?;
    let now = now_millis();
    let id = if let Some((id, existing_alias, existing_email)) = existing {
        let alias = if requested_alias.is_empty() {
            if existing_alias.trim().is_empty() || existing_alias == existing_email {
                oauth_alias("", &identity)
            } else {
                existing_alias
            }
        } else {
            requested_alias.trim().to_string()
        };
        transaction
            .execute(
                "UPDATE accounts SET account_type = 'oauth', api_base_url = NULL, account_id = ?1, chatgpt_user_id = ?2, email = ?3, alias = ?4, plan_type = CASE WHEN ?5 = '' THEN plan_type ELSE ?5 END, auth_json = ?6, oauth_invalidated_at = NULL, updated_at = ?7 WHERE id = ?8 AND product = 'codex'",
                params![identity.account_id, user_id, identity.email, alias, identity.plan_type, auth_json, now, id],
            )
            .map_err(database_error)?;
        id
    } else {
        let id = Uuid::new_v4().to_string();
        let alias = oauth_alias(requested_alias, &identity);
        transaction
            .execute(
                "INSERT INTO accounts (id, product, account_type, api_base_url, account_id, chatgpt_user_id, email, alias, plan_type, auth_json, created_at, updated_at, last_used_at, sort_order) VALUES (?1, 'codex', 'oauth', NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, NULL, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'codex'), 0))",
                params![id, identity.account_id, user_id, identity.email, alias, identity.plan_type, auth_json, now],
            )
            .map_err(database_error)?;
        id
    };
    transaction.commit().map_err(database_error)?;
    let (_, active_id) = resolve_auth_state(&connection, &auth_path(state)?)?;
    get_profile_summary(&connection, &id, active_id.as_deref())
}

pub(crate) fn oauth_alias(requested_alias: &str, identity: &Identity) -> String {
    [requested_alias, &identity.name, &identity.email]
        .into_iter()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or("导入的账户")
        .to_string()
}

pub(crate) fn upsert_relay_profile(
    state: &AppState,
    api_key: &str,
    api_base_url: &str,
    requested_alias: &str,
    upstream_protocol: UpstreamProtocol,
    upstream_auth_mode: UpstreamAuthMode,
    anthropic_max_tokens: i64,
) -> Result<ProfileSummary, String> {
    validate_gateway_settings(upstream_protocol, anthropic_max_tokens)?;
    let api_base_url = normalize_api_base_url(api_base_url)?;
    let auth_json = build_relay_auth_json(api_key)?;
    let fingerprint = credential_fingerprint(api_key);
    let mut connection = open_database(state)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let existing = transaction
        .query_row(
            "SELECT id, alias FROM accounts WHERE product = 'codex' AND account_type = 'relay' AND api_base_url = ?1 AND account_id = ?2 AND upstream_protocol = ?3 AND upstream_auth_mode = ?4 LIMIT 1",
            params![api_base_url, fingerprint, upstream_protocol.as_str(), upstream_auth_mode.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    let now = now_millis();
    let id = if let Some((id, existing_alias)) = existing {
        let alias = if requested_alias.trim().is_empty() {
            existing_alias
        } else {
            requested_alias.trim().to_string()
        };
        transaction
            .execute(
                "UPDATE accounts SET alias = ?1, auth_json = ?2, api_base_url = ?3, account_id = ?4, upstream_protocol = ?5, upstream_auth_mode = ?6, anthropic_max_tokens = ?7, updated_at = ?8 WHERE id = ?9 AND product = 'codex'",
                params![alias, auth_json, api_base_url, fingerprint, upstream_protocol.as_str(), upstream_auth_mode.as_str(), anthropic_max_tokens, now, id],
            )
            .map_err(database_error)?;
        id
    } else {
        let id = Uuid::new_v4().to_string();
        let alias = relay_alias(requested_alias, &api_base_url);
        transaction
            .execute(
                "INSERT INTO accounts (id, product, account_type, api_base_url, account_id, email, alias, plan_type, auth_json, upstream_protocol, upstream_auth_mode, anthropic_max_tokens, created_at, updated_at, last_used_at, sort_order) VALUES (?1, 'codex', 'relay', ?2, ?3, '', ?4, '', ?5, ?6, ?7, ?8, ?9, ?9, NULL, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'codex'), 0))",
                params![id, api_base_url, fingerprint, alias, auth_json, upstream_protocol.as_str(), upstream_auth_mode.as_str(), anthropic_max_tokens, now],
            )
            .map_err(database_error)?;
        id
    };
    transaction.commit().map_err(database_error)?;
    let (_, active_id) = resolve_auth_state(&connection, &auth_path(state)?)?;
    get_profile_summary(&connection, &id, active_id.as_deref())
}

pub(crate) fn update_relay_profile_internal(
    state: &AppState,
    profile_id: &str,
    requested_alias: &str,
    requested_api_key: Option<&str>,
    requested_api_base_url: &str,
    model_profile_id: Option<&str>,
    default_model_id: Option<&str>,
    upstream_protocol: UpstreamProtocol,
    upstream_auth_mode: UpstreamAuthMode,
    anthropic_max_tokens: i64,
) -> Result<ProfileSummary, String> {
    validate_gateway_settings(upstream_protocol, anthropic_max_tokens)?;
    let api_base_url = normalize_api_base_url(requested_api_base_url)?;
    models::validate_model_selection(
        state,
        AccountProduct::Codex,
        model_profile_id,
        default_model_id,
    )?;
    let mut connection = open_database(state)?;
    let (account_type, existing_auth_json, existing_model_profile_id) = connection
        .query_row(
            "SELECT account_type, auth_json, model_profile_id FROM accounts WHERE id = ?1 AND product = 'codex'",
            params![profile_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type != ACCOUNT_TYPE_RELAY {
        return Err("该账户不是中转站账户。".to_string());
    }
    let api_key = requested_api_key
        .map(str::trim)
        .filter(|api_key| !api_key.is_empty())
        .map(str::to_string)
        .or_else(|| extract_api_key(&existing_auth_json).ok().flatten())
        .ok_or_else(|| "中转站账户缺少 API Key。".to_string())?;
    let auth_json = build_relay_auth_json(&api_key)?;
    let fingerprint = credential_fingerprint(&api_key);
    let alias = relay_alias(requested_alias, &api_base_url);
    let auth_path = auth_path(state)?;
    let (_, active_id) = resolve_auth_state(&connection, &auth_path)?;
    let active = active_id.as_deref() == Some(profile_id);
    let gateway_enabled = gateway::is_enabled(&connection)?;
    if active && upstream_protocol.requires_gateway() && !gateway_enabled {
        return Err("该账号协议需要先启用网关模式。".to_string());
    }
    let backup = if !active {
        None
    } else if gateway_enabled {
        gateway::ensure_available()?;
        Some(apply_gateway_profile_files_with_model(
            state,
            &gateway::ensure_local_api_key(&connection)?,
            model_profile_id,
            default_model_id,
            upstream_protocol,
        )?)
    } else if existing_model_profile_id.is_some() || model_profile_id.is_some() {
        Some(apply_profile_files_with_model(
            state,
            &auth_json,
            ACCOUNT_TYPE_RELAY,
            Some(&api_base_url),
            model_profile_id,
            default_model_id,
            upstream_protocol,
        )?)
    } else {
        Some(apply_profile_files(
            state,
            &auth_json,
            ACCOUNT_TYPE_RELAY,
            Some(&api_base_url),
        )?)
    };
    let database_result = (|| {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let duplicate_exists = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE product = 'codex' AND account_type = 'relay' AND id <> ?1 AND api_base_url = ?2 AND account_id = ?3 AND upstream_protocol = ?4 AND upstream_auth_mode = ?5)",
                params![profile_id, api_base_url, fingerprint, upstream_protocol.as_str(), upstream_auth_mode.as_str()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error)?;
        if duplicate_exists {
            return Err("已存在使用相同 API Key 和地址的中转站账户。".to_string());
        }
        transaction
            .execute(
                "UPDATE accounts SET alias = ?1, api_base_url = ?2, account_id = ?3, auth_json = ?4, model_profile_id = ?5, default_model_id = ?6, upstream_protocol = ?7, upstream_auth_mode = ?8, anthropic_max_tokens = ?9, updated_at = ?10 WHERE id = ?11 AND product = 'codex'",
                params![
                    alias,
                    api_base_url,
                    fingerprint,
                    auth_json,
                    model_profile_id,
                    default_model_id,
                    upstream_protocol.as_str(),
                    upstream_auth_mode.as_str(),
                    anthropic_max_tokens,
                    now_millis(),
                    profile_id,
                ],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)
    })();
    if let Err(error) = database_result {
        if let Some(backup) = backup.as_ref() {
            restore_profile_files(state, backup)?;
        }
        return Err(error);
    }
    let (_, active_id) = resolve_auth_state(&connection, &auth_path)?;
    get_profile_summary(&connection, profile_id, active_id.as_deref())
}

fn validate_gateway_settings(
    protocol: UpstreamProtocol,
    anthropic_max_tokens: i64,
) -> Result<(), String> {
    if protocol == UpstreamProtocol::AnthropicMessages && anthropic_max_tokens <= 0 {
        return Err("最大输出 Tokens 必须是正整数。".to_string());
    }
    if anthropic_max_tokens > i64::from(u32::MAX) {
        return Err("最大输出 Tokens 过大。".to_string());
    }
    Ok(())
}

pub(crate) fn default_gateway_settings(
    upstream_protocol: Option<UpstreamProtocol>,
    upstream_auth_mode: Option<UpstreamAuthMode>,
    anthropic_max_tokens: Option<i64>,
) -> (UpstreamProtocol, UpstreamAuthMode, i64) {
    let protocol = upstream_protocol.unwrap_or_default();
    let auth_mode = if protocol == UpstreamProtocol::AnthropicMessages {
        upstream_auth_mode.unwrap_or_default()
    } else {
        UpstreamAuthMode::Bearer
    };
    (
        protocol,
        auth_mode,
        anthropic_max_tokens.unwrap_or(DEFAULT_ANTHROPIC_MAX_TOKENS),
    )
}

pub(crate) fn relay_alias(requested_alias: &str, api_base_url: &str) -> String {
    if !requested_alias.trim().is_empty() {
        return requested_alias.trim().to_string();
    }
    Url::parse(api_base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "中转站".to_string())
}
