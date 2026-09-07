use super::{
    auth::{
        auth_path, credential_json, merge_openai_codex, openai_codex_credential, pi_agent_dir,
        read_auth_storage, read_json_storage, restore_auth_contents, write_auth_storage,
        write_json_storage, OpenAiCodexCredential, PI_PROVIDER_OPENAI_CODEX,
    },
    lock::PiAuthLock,
};
use crate::{
    features::accounts::{
        get_profile_summary_for_product, list_profiles_for_product,
        oauth::{chatgpt_user_id_from_jwt, decode_jwt_claims, identity_from_jwt},
    },
    platform::{
        db::{credential_fingerprint, database_error, open_database},
        local_web,
        state::{
            now_millis, AccountProduct, AppState, AppStatus, AuthState, Identity, ProfileSummary,
            UsageRefreshResult, ACCOUNT_TYPE_OAUTH,
        },
    },
    products::codex::usage::fetch_account_usage_with_token,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use url::Url;
use uuid::Uuid;

const PI_RELAY_PREFIX: &str = "cortana-relay-";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiRelayModel {
    pub(crate) id: String,
    pub(crate) name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiRelayConfiguration {
    pub(crate) models: Vec<PiRelayModel>,
    pub(crate) default_model_id: String,
}

#[derive(Deserialize, Serialize)]
struct StoredRelayCredential {
    r#type: String,
    key: String,
    models: Vec<PiRelayModel>,
}

pub(crate) fn app_status(app: &tauri::AppHandle, state: &AppState) -> Result<AppStatus, String> {
    let settings = read_json_storage(&settings_path(state), "settings.json")?;
    let default_provider = settings.get("defaultProvider").and_then(Value::as_str);
    if let Some(provider) = default_provider.filter(|value| value.starts_with(PI_RELAY_PREFIX)) {
        let connection = open_database(state)?;
        let active_id = connection
            .query_row(
                "SELECT id FROM accounts WHERE product = 'pi' AND account_type = 'relay' AND provider_key = ?1 LIMIT 1",
                params![provider],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?;
        if let Some(id) = active_id {
            return build_status(
                app,
                state,
                Some(&id),
                None,
                "managed",
                "已由 Cortana 管理。",
            );
        }
    }

    let path = auth_path(state);
    let lock = PiAuthLock::acquire(&path)?;
    let storage = read_auth_storage(&path)?;
    let Some(credential) = openai_codex_credential(&storage)? else {
        drop(lock);
        return build_status(
            app,
            state,
            None,
            None,
            "missing",
            "尚未检测到 OpenAI Codex 登录。",
        );
    };

    let mut connection = open_database(state)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let profile_id = find_profile_by_identity(
        &transaction,
        &credential.account_id,
        &credential_user_id(&credential),
    )?;
    if let Some(id) = profile_id.as_deref() {
        sync_managed_credential(&transaction, id, &credential)?;
    }
    transaction.commit().map_err(database_error)?;
    drop(lock);

    if let Some(id) = profile_id.filter(|_| {
        default_provider.is_none() || default_provider == Some(PI_PROVIDER_OPENAI_CODEX)
    }) {
        build_status(
            app,
            state,
            Some(&id),
            None,
            "managed",
            "已由 Cortana 管理。",
        )
    } else {
        build_status(
            app,
            state,
            None,
            Some(detected_profile(&credential)),
            "unmanaged",
            "当前 Pi 账号尚未纳入 Cortana 管理。",
        )
    }
}

pub(crate) fn import_current_profile(
    state: &AppState,
    alias: Option<String>,
) -> Result<ProfileSummary, String> {
    let path = auth_path(state);
    if !path.exists() {
        return Err("未找到可同步的 Pi OpenAI Codex 登录凭据。".to_string());
    }
    let _lock = PiAuthLock::acquire(&path)?;
    let storage = read_auth_storage(&path)?;
    let credential = openai_codex_credential(&storage)?
        .ok_or_else(|| "未找到可同步的 Pi OpenAI Codex 登录凭据。".to_string())?;
    upsert_credential(state, &credential, alias, true)
}

pub(crate) fn import_codex_profile(
    state: &AppState,
    codex_profile_id: &str,
    alias: Option<String>,
) -> Result<ProfileSummary, String> {
    let connection = open_database(state)?;
    let (auth_json, account_id): (String, String) = connection
        .query_row(
            "SELECT auth_json, account_id FROM accounts WHERE id = ?1 AND product = 'codex' AND account_type = 'oauth'",
            params![codex_profile_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "Codex 账号不存在或不是官方 OAuth 账号。".to_string())?;
    let credential = credential_from_codex_auth(&auth_json, &account_id)?;
    let current = openai_codex_credential(&read_auth_storage(&auth_path(state))?)?;
    let active = current.as_ref().is_some_and(|current| {
        current.account_id == account_id
            && credential_user_id(current) == credential_user_id(&credential)
    });
    upsert_credential(state, &credential, alias, active)
}

fn upsert_credential(
    state: &AppState,
    credential: &OpenAiCodexCredential,
    alias: Option<String>,
    active: bool,
) -> Result<ProfileSummary, String> {
    let auth_json = credential_json(credential)?;
    let identity = credential_identity(credential);
    let fallback_alias = default_alias(&identity.account_id);
    let official_alias = official_alias(&identity);
    let now = now_millis();
    let mut connection = open_database(state)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let user_id = credential_user_id(credential);
    if user_id.is_empty() {
        return Err("Pi OpenAI Codex 凭据缺少用户标识。".to_string());
    }
    let existing = find_profile_by_identity(&transaction, &credential.account_id, &user_id)?;
    let id = if let Some(id) = existing {
        transaction
            .execute(
                "UPDATE accounts SET auth_json = ?1, chatgpt_user_id = ?2, email = ?3, plan_type = ?4, alias = CASE WHEN alias = ?5 THEN ?6 ELSE alias END, updated_at = ?7 WHERE id = ?8",
                params![auth_json, user_id, identity.email, identity.plan_type, fallback_alias, official_alias, now, id],
            )
            .map_err(database_error)?;
        id
    } else {
        let id = Uuid::new_v4().to_string();
        let requested = alias.unwrap_or_default();
        let alias = if requested.trim().is_empty() {
            official_alias
        } else {
            requested.trim().to_string()
        };
        let sort_order: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM accounts WHERE product = 'pi'",
                [],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "INSERT INTO accounts (id, product, provider_key, account_type, account_id, chatgpt_user_id, email, alias, plan_type, auth_json, created_at, updated_at, sort_order) VALUES (?1, 'pi', ?2, 'oauth', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)",
                params![id, PI_PROVIDER_OPENAI_CODEX, credential.account_id, user_id, identity.email, alias, identity.plan_type, auth_json, now, sort_order],
            )
            .map_err(database_error)?;
        id
    };
    transaction.commit().map_err(database_error)?;
    let connection = open_database(state)?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Pi,
        &id,
        active.then_some(id.as_str()),
    )
}

fn credential_from_codex_auth(
    auth_json: &str,
    account_id: &str,
) -> Result<OpenAiCodexCredential, String> {
    let auth: Value =
        serde_json::from_str(auth_json).map_err(|_| "存档的 Codex 凭据已损坏。".to_string())?;
    let tokens = auth
        .get("tokens")
        .and_then(Value::as_object)
        .ok_or_else(|| "Codex 账号缺少 OAuth 凭据。".to_string())?;
    let token = |key| {
        tokens
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("Codex 账号缺少 {key}。"))
    };
    let access = token("access_token")?;
    let refresh = token("refresh_token")?;
    let expires = decode_jwt_claims(&access)
        .and_then(|claims| claims.get("exp").and_then(Value::as_f64))
        .map(|seconds| seconds * 1000.0)
        .ok_or_else(|| "Codex access_token 缺少有效期。".to_string())?;
    super::auth::parse_openai_codex_credential(&serde_json::json!({
        "type": "oauth",
        "access": access,
        "refresh": refresh,
        "expires": expires,
        "accountId": account_id,
    }))
}

pub(crate) fn switch_profile(
    state: &AppState,
    profile_id: &str,
    force: bool,
) -> Result<ProfileSummary, String> {
    let connection = open_database(state)?;
    let account_type = connection
        .query_row(
            "SELECT account_type FROM accounts WHERE id = ?1 AND product = 'pi'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "Pi 账户不存在。".to_string())?;
    if account_type == "relay" {
        return switch_relay_profile(state, profile_id);
    }
    switch_oauth_profile(state, profile_id, force)
}

fn switch_oauth_profile(
    state: &AppState,
    profile_id: &str,
    force: bool,
) -> Result<ProfileSummary, String> {
    let path = auth_path(state);
    let _lock = PiAuthLock::acquire(&path)?;
    let original = match fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("无法读取 Pi auth.json：{error}")),
    };
    let storage = read_auth_storage(&path)?;
    let current = openai_codex_credential(&storage)?;
    let mut connection = open_database(state)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;

    if let Some(current) = current.as_ref() {
        if let Some(id) = find_profile_by_identity(
            &transaction,
            &current.account_id,
            &credential_user_id(current),
        )? {
            sync_managed_credential(&transaction, &id, current)?;
        } else if !force {
            return Err(
                "检测到工具外的 Pi OpenAI Codex 登录变化。请先同步当前账号，或确认后强制切换。"
                    .to_string(),
            );
        }
    }

    let (auth_json, account_id, user_id): (String, String, String) = transaction
        .query_row(
            "SELECT auth_json, account_id, chatgpt_user_id FROM accounts WHERE id = ?1 AND product = 'pi' AND provider_key = ?2 AND account_type = 'oauth'",
            params![profile_id, PI_PROVIDER_OPENAI_CODEX],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "Pi 账户不存在。".to_string())?;
    let value =
        serde_json::from_str(&auth_json).map_err(|_| "存档的 Pi 凭据已损坏。".to_string())?;
    let target = super::auth::parse_openai_codex_credential(&value)?;
    if target.account_id != account_id || credential_user_id(&target) != user_id {
        return Err("存档的 Pi 凭据身份不一致。".to_string());
    }

    let merged = merge_openai_codex(storage, &target)?;
    _lock.ensure_healthy()?;
    if let Err(error) = write_auth_storage(&path, &merged) {
        return Err(rollback_error(&path, original.as_deref(), error));
    }
    if let Err(error) = _lock.ensure_healthy() {
        return Err(rollback_error(&path, original.as_deref(), error));
    }
    if let Err(error) = set_default_model(state, PI_PROVIDER_OPENAI_CODEX, None) {
        return Err(rollback_error(&path, original.as_deref(), error));
    }
    let now = now_millis();
    if let Err(error) = transaction
        .execute(
            "UPDATE accounts SET last_used_at = ?1, updated_at = ?1 WHERE id = ?2 AND product = 'pi'",
            params![now, profile_id],
        )
        .map_err(database_error)
    {
        return Err(rollback_error(&path, original.as_deref(), error));
    }
    if let Err(error) = transaction.commit().map_err(database_error) {
        return Err(rollback_error(&path, original.as_deref(), error));
    }

    let connection = open_database(state)?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Pi,
        profile_id,
        Some(profile_id),
    )
}

pub(crate) fn probe_relay_models(
    api_base_url: &str,
    api_key: &str,
    protocol: crate::features::gateway::UpstreamProtocol,
) -> Result<Vec<PiRelayModel>, String> {
    let base_url = normalize_base_url(api_base_url)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;
    let request = client.get(format!("{base_url}/models"));
    let request = if protocol == crate::features::gateway::UpstreamProtocol::AnthropicMessages {
        request.header("x-api-key", api_key.trim())
    } else {
        request.bearer_auth(api_key.trim())
    };
    let mut response = request
        .send()
        .map_err(|error| format!("获取模型失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("获取模型失败：HTTP {}", response.status()));
    }
    const LIMIT: usize = 2 * 1024 * 1024;
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取模型列表失败：{error}"))?;
    if bytes.len() > LIMIT {
        return Err("远端模型响应过大。".to_string());
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "远端模型响应不是有效的 JSON。".to_string())?;
    let items = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .ok_or_else(|| "远端模型响应缺少 data 或 models 数组。".to_string())?;
    let mut models = items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .or_else(|| item.get("slug"))
                .and_then(Value::as_str)?
                .trim();
            if id.is_empty() {
                return None;
            }
            let name = item
                .get("display_name")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(id);
            Some(PiRelayModel {
                id: id.to_string(),
                name: name.to_string(),
            })
        })
        .take(500)
        .collect::<Vec<_>>();
    models.sort_by_key(|model| model.name.to_lowercase());
    let mut ids = HashSet::new();
    models.retain(|model| ids.insert(model.id.clone()));
    Ok(models)
}

pub(crate) fn add_relay_profile(
    state: &AppState,
    api_key: &str,
    api_base_url: &str,
    alias: &str,
    protocol: crate::features::gateway::UpstreamProtocol,
    models: Vec<PiRelayModel>,
    default_model_id: &str,
) -> Result<ProfileSummary, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("API Key 不能为空。".to_string());
    }
    let base_url = normalize_base_url(api_base_url)?;
    let models = normalize_relay_models(models, default_model_id)?;
    let id = Uuid::new_v4().to_string();
    let provider_key = format!("{PI_RELAY_PREFIX}{id}");
    let alias = relay_alias(alias, &base_url);
    let auth_json = serde_json::to_string(&StoredRelayCredential {
        r#type: "api_key".to_string(),
        key: api_key.to_string(),
        models: models.clone(),
    })
    .map_err(|error| error.to_string())?;
    let now = now_millis();
    let connection = open_database(state)?;
    let sort_order: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM accounts WHERE product = 'pi'",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    connection
        .execute(
            "INSERT INTO accounts (id, product, provider_key, account_type, api_base_url, account_id, alias, auth_json, upstream_protocol, created_at, updated_at, sort_order, default_model_id) VALUES (?1, 'pi', ?2, 'relay', ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, ?10)",
            params![id, provider_key, base_url, credential_fingerprint(api_key), alias, auth_json, protocol.as_str(), now, sort_order, default_model_id.trim()],
        )
        .map_err(database_error)?;
    if let Err(error) =
        write_relay_provider(state, &provider_key, &base_url, api_key, protocol, &models)
    {
        let _ = connection.execute("DELETE FROM accounts WHERE id = ?1", params![id]);
        return Err(error);
    }
    get_profile_summary_for_product(&connection, AccountProduct::Pi, &id, None)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_relay_profile(
    state: &AppState,
    profile_id: &str,
    alias: &str,
    api_key: Option<&str>,
    api_base_url: &str,
    protocol: crate::features::gateway::UpstreamProtocol,
    models: Vec<PiRelayModel>,
    default_model_id: &str,
) -> Result<ProfileSummary, String> {
    let base_url = normalize_base_url(api_base_url)?;
    let models = normalize_relay_models(models, default_model_id)?;
    let connection = open_database(state)?;
    let (provider_key, stored): (String, String) = connection
        .query_row(
            "SELECT provider_key, auth_json FROM accounts WHERE id = ?1 AND product = 'pi' AND account_type = 'relay'",
            params![profile_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "Pi 中转站账户不存在。".to_string())?;
    let mut credential: StoredRelayCredential =
        serde_json::from_str(&stored).map_err(|_| "存档的 Pi 中转站凭据已损坏。".to_string())?;
    if let Some(key) = api_key.map(str::trim).filter(|key| !key.is_empty()) {
        credential.key = key.to_string();
    }
    credential.models = models.clone();
    let auth_json = serde_json::to_string(&credential).map_err(|error| error.to_string())?;
    write_relay_provider(
        state,
        &provider_key,
        &base_url,
        &credential.key,
        protocol,
        &models,
    )?;
    let changed = connection
        .execute(
            "UPDATE accounts SET alias = ?1, api_base_url = ?2, account_id = ?3, auth_json = ?4, upstream_protocol = ?5, default_model_id = ?6, updated_at = ?7 WHERE id = ?8 AND product = 'pi' AND account_type = 'relay'",
            params![relay_alias(alias, &base_url), base_url, credential_fingerprint(&credential.key), auth_json, protocol.as_str(), default_model_id.trim(), now_millis(), profile_id],
        )
        .map_err(database_error)?;
    if changed != 1 {
        return Err("Pi 中转站账户不存在。".to_string());
    }
    if is_default_provider(state, &provider_key)? {
        set_default_model(state, &provider_key, Some(default_model_id.trim()))?;
    }
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Pi,
        profile_id,
        is_default_provider(state, &provider_key)?.then_some(profile_id),
    )
}

pub(crate) fn relay_api_key(state: &AppState, profile_id: &str) -> Result<String, String> {
    Ok(relay_credential(state, profile_id)?.1.key)
}

pub(crate) fn relay_models(
    state: &AppState,
    profile_id: &str,
) -> Result<PiRelayConfiguration, String> {
    let (default_model_id, credential) = relay_credential(state, profile_id)?;
    Ok(PiRelayConfiguration {
        models: credential.models,
        default_model_id,
    })
}

fn relay_credential(
    state: &AppState,
    profile_id: &str,
) -> Result<(String, StoredRelayCredential), String> {
    let connection = open_database(state)?;
    let (default_model, auth_json): (String, String) = connection
        .query_row(
            "SELECT COALESCE(default_model_id, ''), auth_json FROM accounts WHERE id = ?1 AND product = 'pi' AND account_type = 'relay'",
            params![profile_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "Pi 中转站账户不存在。".to_string())?;
    let credential =
        serde_json::from_str(&auth_json).map_err(|_| "存档的 Pi 中转站凭据已损坏。".to_string())?;
    Ok((default_model, credential))
}

fn switch_relay_profile(state: &AppState, profile_id: &str) -> Result<ProfileSummary, String> {
    let connection = open_database(state)?;
    let (provider_key, base_url, protocol, default_model): (String, String, String, String) = connection
        .query_row(
            "SELECT provider_key, api_base_url, upstream_protocol, COALESCE(default_model_id, '') FROM accounts WHERE id = ?1 AND product = 'pi' AND account_type = 'relay'",
            params![profile_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "Pi 中转站账户不存在。".to_string())?;
    let credential = relay_credential(state, profile_id)?.1;
    let protocol = crate::features::gateway::UpstreamProtocol::parse(&protocol)?;
    write_relay_provider(
        state,
        &provider_key,
        &base_url,
        &credential.key,
        protocol,
        &credential.models,
    )?;
    set_default_model(state, &provider_key, Some(&default_model))?;
    connection
        .execute(
            "UPDATE accounts SET last_used_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now_millis(), profile_id],
        )
        .map_err(database_error)?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Pi,
        profile_id,
        Some(profile_id),
    )
}

pub(crate) fn remove_relay_provider(state: &AppState, profile_id: &str) -> Result<(), String> {
    let connection = open_database(state)?;
    let provider_key = connection
        .query_row(
            "SELECT provider_key FROM accounts WHERE id = ?1 AND product = 'pi' AND account_type = 'relay'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?;
    let Some(provider_key) = provider_key else {
        return Ok(());
    };
    let auth_path = auth_path(state);
    let models_path = models_path(state);
    let _auth_lock = PiAuthLock::acquire(&auth_path)?;
    let _models_lock = PiAuthLock::acquire(&models_path)?;
    let original_auth = read_optional_bytes(&auth_path)?;
    let original_models = read_optional_bytes(&models_path)?;
    let mut auth = read_auth_storage(&auth_path)?;
    let mut root = read_json_storage(&models_path, "models.json")?;
    auth.remove(&provider_key);
    root.get_mut("providers")
        .and_then(Value::as_object_mut)
        .map(|providers| providers.remove(&provider_key));
    if let Err(error) =
        write_json_storage(&models_path, &root).and_then(|_| write_auth_storage(&auth_path, &auth))
    {
        restore_pair(
            &auth_path,
            original_auth.as_deref(),
            &models_path,
            original_models.as_deref(),
        )?;
        return Err(error);
    }
    if is_default_provider(state, &provider_key)? {
        clear_default_model(state)?;
    }
    Ok(())
}

fn write_relay_provider(
    state: &AppState,
    provider_key: &str,
    base_url: &str,
    api_key: &str,
    protocol: crate::features::gateway::UpstreamProtocol,
    models: &[PiRelayModel],
) -> Result<(), String> {
    let auth_path = auth_path(state);
    let models_path = models_path(state);
    let _auth_lock = PiAuthLock::acquire(&auth_path)?;
    let _models_lock = PiAuthLock::acquire(&models_path)?;
    let original_auth = read_optional_bytes(&auth_path)?;
    let original_models = read_optional_bytes(&models_path)?;
    let mut auth = read_auth_storage(&auth_path)?;
    let mut root = read_json_storage(&models_path, "models.json")?;
    auth.insert(
        provider_key.to_string(),
        serde_json::json!({ "type": "api_key", "key": api_key }),
    );
    let providers = root
        .entry("providers".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| "Pi models.json 的 providers 不是有效对象。".to_string())?;
    providers.insert(
        provider_key.to_string(),
        serde_json::json!({
            "baseUrl": base_url,
            "api": pi_api(protocol),
            "models": models,
        }),
    );
    if let Err(error) =
        write_json_storage(&models_path, &root).and_then(|_| write_auth_storage(&auth_path, &auth))
    {
        restore_pair(
            &auth_path,
            original_auth.as_deref(),
            &models_path,
            original_models.as_deref(),
        )?;
        return Err(error);
    }
    Ok(())
}

fn read_optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn restore_pair(
    first_path: &Path,
    first: Option<&[u8]>,
    second_path: &Path,
    second: Option<&[u8]>,
) -> Result<(), String> {
    let first_result = restore_auth_contents(first_path, first);
    let second_result = restore_auth_contents(second_path, second);
    first_result.and(second_result).map_err(|_| {
        "Pi 配置写入失败，且自动恢复失败。请立即检查 auth.json 和 models.json。".to_string()
    })
}

fn normalize_relay_models(
    models: Vec<PiRelayModel>,
    default_model_id: &str,
) -> Result<Vec<PiRelayModel>, String> {
    let default_model_id = default_model_id.trim();
    if models.len() > 500 {
        return Err("单个中转站最多保存 500 个模型。".to_string());
    }
    let mut normalized = Vec::new();
    for model in models {
        let id = model.id.trim();
        let name = model.name.trim();
        if id.len() > 512 || name.len() > 512 {
            return Err("模型 ID 或名称过长。".to_string());
        }
        if id.is_empty() || normalized.iter().any(|item: &PiRelayModel| item.id == id) {
            continue;
        }
        normalized.push(PiRelayModel {
            id: id.to_string(),
            name: name.to_string(),
        });
        if normalized.last().is_some_and(|model| model.name.is_empty()) {
            normalized.last_mut().unwrap().name = id.to_string();
        }
    }
    if normalized.is_empty() || !normalized.iter().any(|model| model.id == default_model_id) {
        return Err("请至少添加一个模型，并选择其中一个作为默认模型。".to_string());
    }
    Ok(normalized)
}

fn normalize_base_url(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/');
    let url = Url::parse(value).map_err(|_| "API 地址无效。".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("API 地址必须是有效的 HTTP 或 HTTPS 地址。".to_string());
    }
    Ok(value.to_string())
}

fn relay_alias(alias: &str, base_url: &str) -> String {
    let alias = alias.trim();
    if !alias.is_empty() {
        return alias.to_string();
    }
    Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "Pi 中转站".to_string())
}

fn pi_api(protocol: crate::features::gateway::UpstreamProtocol) -> &'static str {
    match protocol {
        crate::features::gateway::UpstreamProtocol::OpenAiResponses => "openai-responses",
        crate::features::gateway::UpstreamProtocol::OpenAiChatCompletions => "openai-completions",
        crate::features::gateway::UpstreamProtocol::AnthropicMessages => "anthropic-messages",
    }
}

fn models_path(state: &AppState) -> PathBuf {
    pi_agent_dir(state).join("models.json")
}

fn settings_path(state: &AppState) -> PathBuf {
    pi_agent_dir(state).join("settings.json")
}

fn is_default_provider(state: &AppState, provider_key: &str) -> Result<bool, String> {
    Ok(read_json_storage(&settings_path(state), "settings.json")?
        .get("defaultProvider")
        .and_then(Value::as_str)
        == Some(provider_key))
}

fn set_default_model(
    state: &AppState,
    provider_key: &str,
    model_id: Option<&str>,
) -> Result<(), String> {
    let path = settings_path(state);
    let _lock = PiAuthLock::acquire(&path)?;
    let mut settings = read_json_storage(&path, "settings.json")?;
    let provider_changed =
        settings.get("defaultProvider").and_then(Value::as_str) != Some(provider_key);
    settings.insert(
        "defaultProvider".to_string(),
        Value::String(provider_key.to_string()),
    );
    if let Some(model_id) = model_id.filter(|id| !id.is_empty()) {
        settings.insert(
            "defaultModel".to_string(),
            Value::String(model_id.to_string()),
        );
    } else if provider_changed {
        settings.remove("defaultModel");
    }
    write_json_storage(&path, &settings)
}

fn clear_default_model(state: &AppState) -> Result<(), String> {
    let path = settings_path(state);
    let _lock = PiAuthLock::acquire(&path)?;
    let mut settings = read_json_storage(&path, "settings.json")?;
    settings.remove("defaultProvider");
    settings.remove("defaultModel");
    write_json_storage(&path, &settings)
}

pub(crate) fn refresh_profile_usage(
    state: &AppState,
    profile_id: &str,
) -> Result<UsageRefreshResult, String> {
    let connection = open_database(state)?;
    let (auth_json, account_id, user_id): (String, String, String) = connection
        .query_row(
            "SELECT auth_json, account_id, chatgpt_user_id FROM accounts WHERE id = ?1 AND product = 'pi' AND provider_key = ?2 AND account_type = 'oauth'",
            params![profile_id, PI_PROVIDER_OPENAI_CODEX],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "Pi 账户不存在。".to_string())?;
    let credential = super::auth::parse_openai_codex_credential(
        &serde_json::from_str(&auth_json).map_err(|_| "存档的 Pi 凭据已损坏。".to_string())?,
    )?;
    let usage = fetch_account_usage_with_token(&credential.access, &account_id)
        .map_err(|error| error.message)?;
    let now = now_millis();
    connection
        .execute(
            "UPDATE accounts SET plan_type = CASE WHEN ?1 = '' THEN plan_type ELSE ?1 END, usage_primary_percent = ?2, usage_primary_window_minutes = ?3, usage_primary_resets_at = ?4, usage_secondary_percent = ?5, usage_secondary_window_minutes = ?6, usage_secondary_resets_at = ?7, usage_updated_at = ?8, usage_refresh_attempted_at = ?8 WHERE id = ?9 AND product = 'pi'",
            params![
                usage.plan_type,
                usage.primary.as_ref().map(|window| window.used_percent),
                usage.primary.as_ref().and_then(|window| window.window_minutes),
                usage.primary.as_ref().and_then(|window| window.resets_at),
                usage.secondary.as_ref().map(|window| window.used_percent),
                usage.secondary.as_ref().and_then(|window| window.window_minutes),
                usage.secondary.as_ref().and_then(|window| window.resets_at),
                now,
                profile_id,
            ],
        )
        .map_err(database_error)?;
    let current = openai_codex_credential(&read_auth_storage(&auth_path(state))?)?;
    let active_id = current
        .as_ref()
        .is_some_and(|current| {
            current.account_id == account_id && credential_user_id(current) == user_id
        })
        .then_some(profile_id);
    Ok(UsageRefreshResult {
        profile: get_profile_summary_for_product(
            &connection,
            AccountProduct::Pi,
            profile_id,
            active_id,
        )?,
        refreshed: true,
    })
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
    let changed = connection
        .execute(
            "UPDATE accounts SET alias = ?1, updated_at = ?2 WHERE id = ?3 AND product = 'pi' AND provider_key = ?4 AND account_type = 'oauth'",
            params![alias, now_millis(), profile_id, PI_PROVIDER_OPENAI_CODEX],
        )
        .map_err(database_error)?;
    if changed != 1 {
        return Err("Pi 账户不存在。".to_string());
    }
    get_profile_summary_for_product(&connection, AccountProduct::Pi, profile_id, None)
}

fn build_status(
    app: &tauri::AppHandle,
    state: &AppState,
    active_id: Option<&str>,
    detected_profile: Option<ProfileSummary>,
    kind: &str,
    message: &str,
) -> Result<AppStatus, String> {
    let connection = open_database(state)?;
    Ok(AppStatus {
        profiles: list_profiles_for_product(&connection, AccountProduct::Pi, active_id)?,
        detected_profile,
        auth_path: auth_path(state).display().to_string(),
        auth_state: AuthState {
            kind: kind.to_string(),
            message: message.to_string(),
        },
        autostart_enabled: app.autolaunch().is_enabled().unwrap_or(false),
        web_access: local_web::web_access_status(app, state)?,
    })
}

fn find_profile_by_identity(
    connection: &Connection,
    account_id: &str,
    user_id: &str,
) -> Result<Option<String>, String> {
    let exact = connection
        .query_row(
            "SELECT id FROM accounts WHERE product = 'pi' AND provider_key = ?1 AND account_type = 'oauth' AND account_id = ?2 AND chatgpt_user_id = ?3 LIMIT 1",
            params![PI_PROVIDER_OPENAI_CODEX, account_id, user_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?;
    if exact.is_some() || user_id.is_empty() {
        return Ok(exact);
    }
    let legacy = connection
        .query_row(
            "SELECT id, auth_json FROM accounts WHERE product = 'pi' AND provider_key = ?1 AND account_type = 'oauth' AND account_id = ?2 AND chatgpt_user_id = '' LIMIT 1",
            params![PI_PROVIDER_OPENAI_CODEX, account_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    Ok(legacy.and_then(|(id, auth_json)| {
        serde_json::from_str(&auth_json)
            .ok()
            .and_then(|value| super::auth::parse_openai_codex_credential(&value).ok())
            .filter(|credential| credential_user_id(credential) == user_id)
            .map(|_| id)
    }))
}

fn sync_managed_credential(
    transaction: &Transaction<'_>,
    profile_id: &str,
    credential: &OpenAiCodexCredential,
) -> Result<(), String> {
    let auth_json = credential_json(credential)?;
    let identity = credential_identity(credential);
    let user_id = credential_user_id(credential);
    let fallback_alias = default_alias(&identity.account_id);
    let official_alias = official_alias(&identity);
    transaction
        .execute(
            "UPDATE accounts SET auth_json = ?1, chatgpt_user_id = ?2, email = ?3, plan_type = ?4, alias = CASE WHEN alias = ?5 THEN ?6 ELSE alias END, updated_at = ?7 WHERE id = ?8 AND (auth_json <> ?1 OR chatgpt_user_id <> ?2 OR email <> ?3 OR plan_type <> ?4 OR (alias = ?5 AND ?5 <> ?6))",
            params![auth_json, user_id, identity.email, identity.plan_type, fallback_alias, official_alias, now_millis(), profile_id],
        )
        .map_err(database_error)?;
    Ok(())
}

fn credential_user_id(credential: &OpenAiCodexCredential) -> String {
    chatgpt_user_id_from_jwt(&credential.access)
}

fn credential_identity(credential: &OpenAiCodexCredential) -> Identity {
    let mut identity = identity_from_jwt(&credential.access);
    identity.account_id = credential.account_id.clone();
    identity
}

fn official_alias(identity: &Identity) -> String {
    [&identity.name, &identity.email]
        .into_iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_alias(&identity.account_id))
}

fn detected_profile(credential: &OpenAiCodexCredential) -> ProfileSummary {
    let identity = credential_identity(credential);
    ProfileSummary {
        id: "detected".to_string(),
        product: AccountProduct::Pi,
        provider_key: PI_PROVIDER_OPENAI_CODEX.to_string(),
        account_type: ACCOUNT_TYPE_OAUTH.to_string(),
        api_base_url: None,
        upstream_protocol: "openaiResponses".to_string(),
        upstream_auth_mode: "bearer".to_string(),
        anthropic_max_tokens: 16_384,
        account_id: identity.account_id.clone(),
        email: identity.email.clone(),
        alias: official_alias(&identity),
        plan_type: identity.plan_type,
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

fn default_alias(account_id: &str) -> String {
    let tail = account_id
        .char_indices()
        .rev()
        .nth(7)
        .map_or(account_id, |(index, _)| &account_id[index..]);
    format!("OpenAI Codex · {tail}")
}

fn rollback_error(path: &Path, original: Option<&[u8]>, error: String) -> String {
    match restore_auth_contents(path, original) {
        Ok(()) => error,
        Err(_) => format!(
            "Pi 账号切换失败，且认证文件自动恢复失败。请立即检查 {}。",
            path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    #[test]
    fn reads_codex_identity_from_pi_access_token() {
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "jwt-account",
                    "chatgpt_user_id": "user-1",
                    "chatgpt_plan_type": "plus"
                },
                "https://api.openai.com/profile": {
                    "name": "Pi User",
                    "email": "pi@example.com"
                }
            }))
            .unwrap(),
        );
        let credential = super::super::auth::parse_openai_codex_credential(&serde_json::json!({
            "type": "oauth",
            "access": format!("header.{payload}.signature"),
            "refresh": "refresh",
            "expires": 1,
            "accountId": "pi-account"
        }))
        .unwrap();

        let profile = detected_profile(&credential);
        assert_eq!(profile.account_id, "pi-account");
        assert_eq!(profile.alias, "Pi User");
        assert_eq!(profile.email, "pi@example.com");
        assert_eq!(profile.plan_type, "plus");
        assert_eq!(credential_user_id(&credential), "user-1");
    }

    #[test]
    fn stores_users_from_the_same_codex_workspace_separately() {
        let directory = std::env::temp_dir().join(format!("cortana-pi-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        crate::platform::db::initialize_database(&state).unwrap();

        for (user_id, email) in [("user-1", "one@example.com"), ("user-2", "two@example.com")] {
            let payload = URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&serde_json::json!({
                    "https://api.openai.com/auth": {
                        "chatgpt_account_id": "workspace",
                        "chatgpt_user_id": user_id
                    },
                    "https://api.openai.com/profile": { "email": email }
                }))
                .unwrap(),
            );
            let credential =
                super::super::auth::parse_openai_codex_credential(&serde_json::json!({
                    "type": "oauth",
                    "access": format!("header.{payload}.signature"),
                    "refresh": "refresh",
                    "expires": 1,
                    "accountId": "workspace"
                }))
                .unwrap();
            upsert_credential(&state, &credential, None, false).unwrap();
        }

        let profiles =
            list_profiles_for_product(&open_database(&state).unwrap(), AccountProduct::Pi, None)
                .unwrap();
        assert_eq!(profiles.len(), 2);
        assert_ne!(profiles[0].email, profiles[1].email);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn manages_relay_as_a_pi_provider_without_overwriting_other_providers() {
        let directory =
            std::env::temp_dir().join(format!("cortana-pi-relay-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(directory.join(".pi/agent")).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        crate::platform::db::initialize_database(&state).unwrap();
        fs::write(
            auth_path(&state),
            r#"{"other":{"type":"api_key","key":"keep"}}"#,
        )
        .unwrap();
        fs::write(
            models_path(&state),
            r#"{"providers":{"other":{"models":[]}}}"#,
        )
        .unwrap();
        fs::write(settings_path(&state), r#"{"theme":"dark"}"#).unwrap();

        let profile = add_relay_profile(
            &state,
            "secret",
            "https://relay.example/v1/",
            "Relay",
            crate::features::gateway::UpstreamProtocol::OpenAiResponses,
            vec![PiRelayModel {
                id: "gpt-test".to_string(),
                name: "GPT Test".to_string(),
            }],
            "gpt-test",
        )
        .unwrap();
        assert_eq!(profile.account_type, "relay");
        assert_eq!(
            profile.api_base_url.as_deref(),
            Some("https://relay.example/v1")
        );
        let auth = read_auth_storage(&auth_path(&state)).unwrap();
        assert_eq!(auth["other"]["key"], "keep");
        assert_eq!(auth[&profile.provider_key]["key"], "secret");
        let models = read_json_storage(&models_path(&state), "models.json").unwrap();
        assert!(models["providers"].get("other").is_some());
        assert_eq!(
            models["providers"][&profile.provider_key]["api"],
            "openai-responses"
        );

        switch_profile(&state, &profile.id, false).unwrap();
        let settings = read_json_storage(&settings_path(&state), "settings.json").unwrap();
        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["defaultProvider"], profile.provider_key);
        assert_eq!(settings["defaultModel"], "gpt-test");

        remove_relay_provider(&state, &profile.id).unwrap();
        let auth = read_auth_storage(&auth_path(&state)).unwrap();
        let models = read_json_storage(&models_path(&state), "models.json").unwrap();
        assert!(auth.get(&profile.provider_key).is_none());
        assert!(models["providers"].get(&profile.provider_key).is_none());
        assert!(models["providers"].get("other").is_some());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn preserves_default_model_only_when_staying_with_the_same_provider() {
        let directory =
            std::env::temp_dir().join(format!("cortana-pi-settings-test-{}", Uuid::new_v4()));
        fs::create_dir_all(directory.join(".pi/agent")).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        fs::write(settings_path(&state), r#"{"theme":"dark"}"#).unwrap();

        set_default_model(&state, PI_PROVIDER_OPENAI_CODEX, None).unwrap();
        let settings = read_json_storage(&settings_path(&state), "settings.json").unwrap();
        assert_eq!(settings["defaultProvider"], PI_PROVIDER_OPENAI_CODEX);
        assert!(!settings.contains_key("defaultModel"));

        set_default_model(&state, PI_PROVIDER_OPENAI_CODEX, Some("gpt-6-astra")).unwrap();
        set_default_model(&state, PI_PROVIDER_OPENAI_CODEX, None).unwrap();
        let settings = read_json_storage(&settings_path(&state), "settings.json").unwrap();
        assert_eq!(settings["defaultModel"], "gpt-6-astra");
        assert_eq!(settings["theme"], "dark");

        set_default_model(&state, "relay", Some("relay-model")).unwrap();
        set_default_model(&state, PI_PROVIDER_OPENAI_CODEX, None).unwrap();
        let settings = read_json_storage(&settings_path(&state), "settings.json").unwrap();
        assert_eq!(settings["defaultProvider"], PI_PROVIDER_OPENAI_CODEX);
        assert!(!settings.contains_key("defaultModel"));
        assert_eq!(settings["theme"], "dark");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn converts_codex_auth_to_pi_credential() {
        let payload = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&serde_json::json!({ "exp": 1_800_000_000 })).unwrap());
        let credential = credential_from_codex_auth(
            &serde_json::json!({
                "tokens": {
                    "access_token": format!("header.{payload}.signature"),
                    "refresh_token": "refresh"
                }
            })
            .to_string(),
            "account",
        )
        .unwrap();

        assert_eq!(credential.account_id, "account");
        assert_eq!(credential.refresh, "refresh");
        assert_eq!(credential.expires, 1_800_000_000_000.0);
    }
}
