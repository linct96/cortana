use super::{
    antigravity, claude,
    codex::*,
    db::*,
    grok,
    oauth::{chatgpt_user_id_from_auth_json, identity_from_auth_json},
    tray::*,
    *,
};

#[tauri::command]
pub(super) async fn get_app_status(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    product: Option<AccountProduct>,
) -> Result<AppStatus, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match product.unwrap_or_default() {
        AccountProduct::Codex => app_status(&app, &state),
        AccountProduct::Claude => claude::app_status(&app, &state),
        AccountProduct::Antigravity => antigravity::app_status(&app, &state),
        AccountProduct::Grok => grok::app_status(&app, &state),
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn switch_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    force: bool,
    product: Option<AccountProduct>,
) -> Result<ProfileSummary, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let profile = match product.unwrap_or_default() {
            AccountProduct::Codex => switch_profile_internal(&state, &profile_id, force)?,
            AccountProduct::Claude => claude::switch_profile(&state, &profile_id, force)?,
            AccountProduct::Antigravity => antigravity::switch_profile(&state, &profile_id, force)?,
            AccountProduct::Grok => grok::switch_profile(&state, &profile_id, force)?,
        };
        refresh_tray(&app)?;
        Ok(profile)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn import_current_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    alias: Option<String>,
    product: Option<AccountProduct>,
) -> Result<ProfileSummary, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let product = product.unwrap_or_default();
        if product != AccountProduct::Codex {
            let profile = match product {
                AccountProduct::Claude => claude::import_current_profile(&state, alias)?,
                AccountProduct::Antigravity => antigravity::import_current_profile(&state, alias)?,
                AccountProduct::Grok => grok::import_current_profile(&state, alias)?,
                AccountProduct::Codex => unreachable!(),
            };
            refresh_tray(&app)?;
            return Ok(profile);
        }
        let auth_path = auth_path(&state)?;
        let auth_json = read_auth_json(&auth_path)?
            .ok_or_else(|| "未找到 Codex 的 auth.json，无法导入。".to_string())?;
        let requested_alias = alias.unwrap_or_default();
        let profile = if let Some(api_key) = extract_api_key(&auth_json)? {
            let (_, provider_name, api_base_url) =
                read_provider_config(&codex_config_path(&state)?)?;
            let api_base_url = api_base_url.ok_or_else(|| {
                "当前 API Key 登录缺少中转站 API 地址，请先补充地址。".to_string()
            })?;
            let alias = if requested_alias.trim().is_empty() {
                provider_name.as_deref().unwrap_or_default()
            } else {
                requested_alias.trim()
            };
            let profile = upsert_relay_profile(&state, &api_key, &api_base_url, alias)?;
            switch_profile_internal(&state, &profile.id, true)?
        } else {
            let profile = upsert_profile_from_auth(&state, &auth_json, requested_alias.trim())?;
            switch_profile_internal(&state, &profile.id, true)?
        };
        refresh_tray(&app)?;
        if profile.account_type == ACCOUNT_TYPE_OAUTH {
            Ok(refresh_profile_usage_internal(&state, &profile.id).unwrap_or(profile))
        } else {
            Ok(profile)
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) fn add_relay_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    api_key: String,
    api_base_url: String,
    alias: Option<String>,
    activate: bool,
    product: Option<AccountProduct>,
) -> Result<ProfileSummary, String> {
    let product = product.unwrap_or_default();
    let profile = match product {
        AccountProduct::Codex => upsert_relay_profile(
            &state,
            &api_key,
            &api_base_url,
            alias.unwrap_or_default().trim(),
        )?,
        AccountProduct::Claude => claude::upsert_relay_profile(
            &state,
            &api_key,
            &api_base_url,
            alias.unwrap_or_default().trim(),
            "manual",
        )?,
        AccountProduct::Antigravity | AccountProduct::Grok => {
            return Err("该产品暂不支持中转站账户。".to_string());
        }
    };
    let profile = if activate {
        match product {
            AccountProduct::Codex => switch_profile_internal(&state, &profile.id, true)?,
            AccountProduct::Claude => claude::switch_profile(&state, &profile.id, true)?,
            AccountProduct::Antigravity | AccountProduct::Grok => unreachable!(),
        }
    } else {
        profile
    };
    refresh_tray(&app)?;
    Ok(profile)
}

#[tauri::command]
pub(super) async fn refresh_profile_usage(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<ProfileSummary, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = open_database(&state)?;
        let product = connection
            .query_row(
                "SELECT product FROM accounts WHERE id = ?1",
                params![profile_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| "账户不存在。".to_string())?;
        match product.as_str() {
            "codex" => refresh_profile_usage_internal(&state, &profile_id),
            "claude" => claude::refresh_profile_usage(&state, &profile_id),
            "antigravity" => antigravity::refresh_profile_usage(&state, &profile_id),
            "grok" => grok::refresh_profile_usage(&state, &profile_id),
            _ => Err("不支持该账户类型。".to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn get_profile_reset_credits(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<ResetCredits, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = open_database(&state)?;
        let (account_type, account_id, auth_json) = connection
            .query_row(
                "SELECT account_type, account_id, auth_json FROM accounts WHERE id = ?1 AND product = 'codex'",
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
            return Err("中转站账户不支持重置卡查询。".to_string());
        }
        let credits = fetch_reset_credits(&auth_json, &account_id)?;
        connection
            .execute(
                "UPDATE accounts SET reset_credits_available_count = ?1 WHERE id = ?2 AND product = 'codex'",
                params![credits.available_count, profile_id],
            )
            .map_err(database_error)?;
        Ok(credits)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) fn get_profile_auth(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<String, String> {
    let connection = open_database(&state)?;
    get_profile_auth_json(&connection, &profile_id)
}

pub(super) fn get_profile_auth_json(
    connection: &Connection,
    profile_id: &str,
) -> Result<String, String> {
    let profile = connection
        .query_row(
            "SELECT account_type, auth_json FROM accounts WHERE id = ?1 AND product = 'codex'",
            params![profile_id],
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

#[tauri::command]
pub(super) fn update_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    alias: String,
    auth_json: Option<String>,
    product: Option<AccountProduct>,
) -> Result<ProfileSummary, String> {
    let profile = match product.unwrap_or_default() {
        AccountProduct::Codex => update_profile_internal(
            &state,
            &profile_id,
            &alias,
            auth_json
                .as_deref()
                .ok_or_else(|| "缺少 auth.json。".to_string())?,
        )?,
        AccountProduct::Claude => claude::update_alias(&state, &profile_id, &alias)?,
        AccountProduct::Antigravity => antigravity::update_alias(&state, &profile_id, &alias)?,
        AccountProduct::Grok => grok::update_alias(&state, &profile_id, &alias)?,
    };
    refresh_tray(&app)?;
    Ok(profile)
}

#[tauri::command]
pub(super) fn update_relay_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    alias: String,
    api_key: Option<String>,
    api_base_url: String,
    product: Option<AccountProduct>,
) -> Result<ProfileSummary, String> {
    let profile = match product.unwrap_or_default() {
        AccountProduct::Codex => update_relay_profile_internal(
            &state,
            &profile_id,
            &alias,
            api_key.as_deref(),
            &api_base_url,
        )?,
        AccountProduct::Claude => claude::update_relay_profile(
            &state,
            &profile_id,
            &alias,
            api_key.as_deref(),
            &api_base_url,
        )?,
        AccountProduct::Antigravity | AccountProduct::Grok => {
            return Err("该产品暂不支持中转站账户。".to_string());
        }
    };
    refresh_tray(&app)?;
    Ok(profile)
}

#[tauri::command]
pub(super) fn reorder_profiles(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    profile_ids: Vec<String>,
    product: Option<AccountProduct>,
) -> Result<(), String> {
    let product = product.unwrap_or_default();
    let mut connection = open_database(&state)?;
    let mut existing_ids = list_profiles_for_product(&connection, product, None)?
        .into_iter()
        .map(|profile| profile.id)
        .collect::<Vec<_>>();
    let mut received_ids = profile_ids.clone();
    existing_ids.sort();
    received_ids.sort();
    if existing_ids != received_ids {
        return Err("账户排序数据无效。".to_string());
    }
    let transaction = connection.transaction().map_err(database_error)?;
    for (sort_order, profile_id) in profile_ids.iter().enumerate() {
        transaction
            .execute(
                "UPDATE accounts SET sort_order = ?1 WHERE id = ?2 AND product = ?3",
                params![sort_order, profile_id, product.as_str()],
            )
            .map_err(database_error)?;
    }
    transaction.commit().map_err(database_error)?;
    refresh_tray(&app)
}

#[tauri::command]
pub(super) fn delete_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    product: Option<AccountProduct>,
) -> Result<(), String> {
    let product = product.unwrap_or_default();
    let connection = open_database(&state)?;
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1 AND product = ?2)",
            params![profile_id, product.as_str()],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    drop(connection);
    if !exists {
        return Err("账户不存在。".to_string());
    }
    if product == AccountProduct::Claude {
        claude::clear_active_profile(&state, &profile_id)?;
    }
    let mut connection = open_database(&state)?;
    let transaction = connection.transaction().map_err(database_error)?;
    let changed = transaction
        .execute(
            "DELETE FROM accounts WHERE id = ?1 AND product = ?2",
            params![profile_id, product.as_str()],
        )
        .map_err(database_error)?;
    debug_assert_eq!(changed, 1);
    if product == AccountProduct::Antigravity {
        antigravity::clear_active_profile(&transaction, &profile_id)?;
    }
    transaction.commit().map_err(database_error)?;
    refresh_tray(&app)
}

#[tauri::command]
pub(super) fn set_codex_home(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    codex_home: String,
) -> Result<AppStatus, String> {
    let trimmed = codex_home.trim();
    let connection = open_database(&state)?;
    set_setting(&connection, "codex_home", trimmed)?;
    refresh_tray(&app)?;
    app_status(&app, &state)
}

pub(super) fn get_active_product(state: State<'_, AppState>) -> Result<AccountProduct, String> {
    let connection = open_database(&state)?;
    Ok(
        match get_setting(&connection, "active_product")?.as_deref() {
            Some("claude") => AccountProduct::Claude,
            Some("antigravity") => AccountProduct::Antigravity,
            Some("grok") => AccountProduct::Grok,
            _ => AccountProduct::Codex,
        },
    )
}

pub(super) fn set_active_product(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    product: AccountProduct,
) -> Result<(), String> {
    let connection = open_database(&state)?;
    set_setting(&connection, "active_product", product.as_str())?;
    refresh_tray(&app)
}

pub(super) fn app_status(app: &tauri::AppHandle, state: &AppState) -> Result<AppStatus, String> {
    let connection = open_database(state)?;
    let auth_path = auth_path(state)?;
    let (auth_state, active_profile_id) = resolve_auth_state(&connection, &auth_path)?;
    let profiles = list_profiles(&connection, active_profile_id.as_deref())?;
    let detected_profile = active_profile_id
        .is_none()
        .then(|| read_auth_json(&auth_path))
        .transpose()?
        .flatten()
        .filter(|auth_json| has_usable_credential(auth_json))
        .map(|auth_json| {
            detected_profile_from_auth(auth_json.as_str(), &auth_path.with_file_name("config.toml"))
        })
        .transpose()?
        .flatten();
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

pub(super) fn list_profiles(
    connection: &Connection,
    active_id: Option<&str>,
) -> Result<Vec<ProfileSummary>, String> {
    list_profiles_for_product(connection, AccountProduct::Codex, active_id)
}

pub(super) fn list_profiles_for_product(
    connection: &Connection,
    product: AccountProduct,
    active_id: Option<&str>,
) -> Result<Vec<ProfileSummary>, String> {
    let mut statement = connection
        .prepare("SELECT id, account_type, api_base_url, account_id, email, alias, plan_type, usage_primary_percent, usage_primary_window_minutes, usage_primary_resets_at, usage_secondary_percent, usage_secondary_window_minutes, usage_secondary_resets_at, usage_updated_at, last_used_at, updated_at, reset_credits_available_count, antigravity_quota_json, auth_json FROM accounts WHERE product = ?1 ORDER BY sort_order ASC, created_at ASC")
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

pub(super) fn profile_summary_from_row(
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
        is_renewable,
        last_used_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

pub(super) fn get_profile_summary(
    connection: &Connection,
    profile_id: &str,
    active_id: Option<&str>,
) -> Result<ProfileSummary, String> {
    get_profile_summary_for_product(connection, AccountProduct::Codex, profile_id, active_id)
}

pub(super) fn get_profile_summary_for_product(
    connection: &Connection,
    product: AccountProduct,
    profile_id: &str,
    active_id: Option<&str>,
) -> Result<ProfileSummary, String> {
    connection
        .query_row(
            "SELECT id, account_type, api_base_url, account_id, email, alias, plan_type, usage_primary_percent, usage_primary_window_minutes, usage_primary_resets_at, usage_secondary_percent, usage_secondary_window_minutes, usage_secondary_resets_at, usage_updated_at, last_used_at, updated_at, reset_credits_available_count, antigravity_quota_json, auth_json FROM accounts WHERE id = ?1 AND product = ?2",
            params![profile_id, product.as_str()],
            |row| profile_summary_from_row(row, product, active_id),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())
}

pub(super) fn detected_profile_from_auth(
    auth_json: &str,
    config_path: &Path,
) -> Result<Option<ProfileSummary>, String> {
    let auth: Value =
        serde_json::from_str(auth_json).map_err(|_| "auth.json 不是有效的 JSON。".to_string())?;
    if extract_api_key_from_value(&auth).is_some() {
        let (_, provider_name, api_base_url) = read_provider_config(config_path)?;
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
        let now = now_millis();
        return Ok(Some(ProfileSummary {
            id: "detected".to_string(),
            product: AccountProduct::Codex,
            account_type: ACCOUNT_TYPE_RELAY.to_string(),
            api_base_url,
            account_id: String::new(),
            email: String::new(),
            alias,
            plan_type: String::new(),
            usage_primary: None,
            usage_secondary: None,
            antigravity_quota: None,
            usage_updated_at: None,
            reset_credits_available_count: None,
            is_renewable: true,
            is_active: true,
            last_used_at: None,
            updated_at: now,
        }));
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
        is_renewable: true,
        is_active: true,
        last_used_at: None,
        updated_at: now,
    }))
}

pub(super) fn profile_id_for_auth(
    connection: &Connection,
    auth_json: &str,
    config_path: &Path,
) -> Result<Option<String>, String> {
    if let Some(api_key) = extract_api_key(auth_json)? {
        let (_, _, api_base_url) = read_provider_config(config_path)?;
        let Some(api_base_url) = api_base_url.and_then(|url| normalize_api_base_url(&url).ok())
        else {
            return Ok(None);
        };
        return connection
            .query_row(
                "SELECT id FROM accounts WHERE product = 'codex' AND account_type = 'relay' AND api_base_url = ?1 AND trim(COALESCE(json_extract(auth_json, '$.OPENAI_API_KEY'), '')) = ?2 LIMIT 1",
                params![api_base_url, api_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error);
    }
    let Ok(auth) = serde_json::from_str(auth_json) else {
        return Ok(None);
    };
    let identity = identity_from_auth_json(&auth);
    let user_id = chatgpt_user_id_from_auth_json(&auth);
    Ok(find_codex_oauth_profile(connection, &identity.account_id, &user_id)?.map(|(id, _, _)| id))
}

fn find_codex_oauth_profile(
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

pub(super) fn resolve_auth_state(
    connection: &Connection,
    path: &Path,
) -> Result<(AuthState, Option<String>), String> {
    let Some(auth_json) = read_auth_json(path)? else {
        return Ok((
            AuthState {
                kind: "missing".to_string(),
                message: "尚未检测到 Codex 登录文件。".to_string(),
            },
            None,
        ));
    };
    let profile_id =
        profile_id_for_auth(connection, &auth_json, &path.with_file_name("config.toml"))?;
    if let Some(profile_id) = profile_id.as_deref() {
        connection
            .execute(
                "UPDATE accounts SET auth_json = ?1, updated_at = ?2 WHERE id = ?3 AND product = 'codex' AND account_type = 'oauth' AND auth_json <> ?1",
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

pub(super) fn switch_profile_internal(
    state: &AppState,
    profile_id: &str,
    force: bool,
) -> Result<ProfileSummary, String> {
    let mut connection = open_database(state)?;
    let path = auth_path(state)?;
    let (_, active_id) = resolve_auth_state(&connection, &path)?;
    let external_auth_has_credential =
        read_auth_json(&path)?.is_some_and(|auth_json| has_usable_credential(&auth_json));
    if active_id.is_none() && external_auth_has_credential && !force {
        return Err(
            "检测到工具外的 Codex 登录或 API 配置变更。请先导入当前状态，或确认后强制切换。"
                .to_string(),
        );
    }
    let row = connection
        .query_row(
            "SELECT id, account_type, api_base_url, auth_json FROM accounts WHERE id = ?1 AND product = 'codex'",
            params![profile_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    let backup = apply_profile_files(state, &row.3, &row.1, row.2.as_deref())?;
    let now = now_millis();
    let database_result = (|| {
        let transaction = connection.transaction().map_err(database_error)?;
        transaction
            .execute(
                "UPDATE accounts SET last_used_at = ?1, updated_at = ?1 WHERE id = ?2 AND product = 'codex'",
                params![now, row.0],
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

pub(super) fn update_profile_internal(
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
    if find_codex_oauth_profile(&connection, &identity.account_id, &user_id)?
        .is_some_and(|(id, _, _)| id != profile_id)
    {
        return Err("该 Codex 账号已存在。".to_string());
    }
    if identity.email.is_empty() {
        identity.email = existing_email;
    }
    let alias = oauth_alias(requested_alias, &identity);
    let auth_path = auth_path(state)?;
    let (_, active_id) = resolve_auth_state(&connection, &auth_path)?;
    let active = active_id.as_deref() == Some(profile_id);
    let backup = active
        .then(|| apply_profile_files(state, &formatted_auth_json, ACCOUNT_TYPE_OAUTH, None))
        .transpose()?;
    let database_result = (|| {
        let transaction = connection.transaction().map_err(database_error)?;
        let changed = transaction
            .execute(
                "UPDATE accounts SET account_id = ?1, chatgpt_user_id = ?2, email = ?3, alias = ?4, plan_type = CASE WHEN ?5 = '' THEN plan_type ELSE ?5 END, auth_json = ?6, updated_at = ?7 WHERE id = ?8 AND product = 'codex'",
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

pub(super) fn upsert_profile_from_auth(
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
    let connection = open_database(state)?;
    let existing = find_codex_oauth_profile(&connection, &identity.account_id, &user_id)?;
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
        connection
            .execute(
                "UPDATE accounts SET account_type = 'oauth', api_base_url = NULL, account_id = ?1, chatgpt_user_id = ?2, email = ?3, alias = ?4, plan_type = CASE WHEN ?5 = '' THEN plan_type ELSE ?5 END, auth_json = ?6, updated_at = ?7 WHERE id = ?8 AND product = 'codex'",
                params![identity.account_id, user_id, identity.email, alias, identity.plan_type, auth_json, now, id],
            )
            .map_err(database_error)?;
        id
    } else {
        let id = Uuid::new_v4().to_string();
        let alias = oauth_alias(requested_alias, &identity);
        connection
            .execute(
                "INSERT INTO accounts (id, product, account_type, api_base_url, account_id, chatgpt_user_id, email, alias, plan_type, auth_json, created_at, updated_at, last_used_at, sort_order) VALUES (?1, 'codex', 'oauth', NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, NULL, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'codex'), 0))",
                params![id, identity.account_id, user_id, identity.email, alias, identity.plan_type, auth_json, now],
            )
            .map_err(database_error)?;
        id
    };
    let (_, active_id) = resolve_auth_state(&connection, &auth_path(state)?)?;
    get_profile_summary(&connection, &id, active_id.as_deref())
}

pub(super) fn oauth_alias(requested_alias: &str, identity: &Identity) -> String {
    [requested_alias, &identity.name, &identity.email]
        .into_iter()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or("导入的账户")
        .to_string()
}

pub(super) fn upsert_relay_profile(
    state: &AppState,
    api_key: &str,
    api_base_url: &str,
    requested_alias: &str,
) -> Result<ProfileSummary, String> {
    let api_base_url = normalize_api_base_url(api_base_url)?;
    let auth_json = build_relay_auth_json(api_key)?;
    let connection = open_database(state)?;
    let existing = connection
        .query_row(
            "SELECT id, alias FROM accounts WHERE product = 'codex' AND account_type = 'relay' AND api_base_url = ?1 AND trim(COALESCE(json_extract(auth_json, '$.OPENAI_API_KEY'), '')) = ?2 LIMIT 1",
            params![api_base_url, api_key.trim()],
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
        connection
            .execute(
                "UPDATE accounts SET alias = ?1, auth_json = ?2, api_base_url = ?3, updated_at = ?4 WHERE id = ?5 AND product = 'codex'",
                params![alias, auth_json, api_base_url, now, id],
            )
            .map_err(database_error)?;
        id
    } else {
        let id = Uuid::new_v4().to_string();
        let alias = relay_alias(requested_alias, &api_base_url);
        connection
            .execute(
                "INSERT INTO accounts (id, product, account_type, api_base_url, account_id, email, alias, plan_type, auth_json, created_at, updated_at, last_used_at, sort_order) VALUES (?1, 'codex', 'relay', ?2, '', '', ?3, '', ?4, ?5, ?5, NULL, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'codex'), 0))",
                params![id, api_base_url, alias, auth_json, now],
            )
            .map_err(database_error)?;
        id
    };
    let (_, active_id) = resolve_auth_state(&connection, &auth_path(state)?)?;
    get_profile_summary(&connection, &id, active_id.as_deref())
}

pub(super) fn update_relay_profile_internal(
    state: &AppState,
    profile_id: &str,
    requested_alias: &str,
    requested_api_key: Option<&str>,
    requested_api_base_url: &str,
) -> Result<ProfileSummary, String> {
    let api_base_url = normalize_api_base_url(requested_api_base_url)?;
    let mut connection = open_database(state)?;
    let (account_type, existing_auth_json) = connection
        .query_row(
            "SELECT account_type, auth_json FROM accounts WHERE id = ?1 AND product = 'codex'",
            params![profile_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
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
    let duplicate_exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE product = 'codex' AND account_type = 'relay' AND id <> ?1 AND api_base_url = ?2 AND trim(COALESCE(json_extract(auth_json, '$.OPENAI_API_KEY'), '')) = ?3)",
            params![profile_id, api_base_url, api_key],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if duplicate_exists {
        return Err("已存在使用相同 API Key 和地址的中转站账户。".to_string());
    }
    let alias = relay_alias(requested_alias, &api_base_url);
    let auth_path = auth_path(state)?;
    let (_, active_id) = resolve_auth_state(&connection, &auth_path)?;
    let active = active_id.as_deref() == Some(profile_id);
    let backup = active
        .then(|| apply_profile_files(state, &auth_json, ACCOUNT_TYPE_RELAY, Some(&api_base_url)))
        .transpose()?;
    let database_result = (|| {
        let transaction = connection.transaction().map_err(database_error)?;
        transaction
            .execute(
                "UPDATE accounts SET alias = ?1, api_base_url = ?2, auth_json = ?3, updated_at = ?4 WHERE id = ?5 AND product = 'codex'",
                params![alias, api_base_url, auth_json, now_millis(), profile_id],
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

pub(super) fn relay_alias(requested_alias: &str, api_base_url: &str) -> String {
    if !requested_alias.trim().is_empty() {
        return requested_alias.trim().to_string();
    }
    Url::parse(api_base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "中转站".to_string())
}

#[derive(Debug)]
pub(super) struct AccountUsage {
    pub(super) plan_type: String,
    pub(super) primary: Option<UsageWindow>,
    pub(super) secondary: Option<UsageWindow>,
}

pub(super) fn refresh_profile_usage_internal(
    state: &AppState,
    profile_id: &str,
) -> Result<ProfileSummary, String> {
    let connection = open_database(state)?;
    let (account_type, account_id, auth_json) = connection
        .query_row(
            "SELECT account_type, account_id, auth_json FROM accounts WHERE id = ?1 AND product = 'codex'",
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
    if account_type == ACCOUNT_TYPE_RELAY {
        return Err("中转站账户不支持额度查询。".to_string());
    }
    let usage = fetch_account_usage(&auth_json, &account_id)?;
    let plan_type = usage.plan_type.trim().to_lowercase();
    let reset_credits = (!plan_type.is_empty() && plan_type != "free")
        .then(|| fetch_reset_credits(&auth_json, &account_id))
        .transpose()?;
    let now = now_millis();
    connection
        .execute(
            "UPDATE accounts SET plan_type = CASE WHEN ?1 = '' THEN plan_type ELSE ?1 END, usage_primary_percent = ?2, usage_primary_window_minutes = ?3, usage_primary_resets_at = ?4, usage_secondary_percent = ?5, usage_secondary_window_minutes = ?6, usage_secondary_resets_at = ?7, usage_updated_at = ?8, reset_credits_available_count = ?9 WHERE id = ?10 AND product = 'codex'",
            params![
                usage.plan_type,
                usage.primary.as_ref().map(|window| window.used_percent),
                usage.primary.as_ref().and_then(|window| window.window_minutes),
                usage.primary.as_ref().and_then(|window| window.resets_at),
                usage.secondary.as_ref().map(|window| window.used_percent),
                usage.secondary.as_ref().and_then(|window| window.window_minutes),
                usage.secondary.as_ref().and_then(|window| window.resets_at),
                now,
                reset_credits.as_ref().map(|credits| credits.available_count),
                profile_id,
            ],
        )
        .map_err(database_error)?;
    let (_, active_id) = resolve_auth_state(&connection, &auth_path(state)?)?;
    get_profile_summary(&connection, profile_id, active_id.as_deref())
}

#[derive(Deserialize)]
struct ResetCreditsResponse {
    available_count: i64,
    credits: Vec<ResetCreditResponse>,
}

#[derive(Deserialize)]
struct ResetCreditResponse {
    id: String,
    title: String,
    status: String,
    expires_at: String,
    granted_at: String,
}

pub(super) fn fetch_reset_credits(
    auth_json: &str,
    account_id: &str,
) -> Result<ResetCredits, String> {
    let auth: Value =
        serde_json::from_str(auth_json).map_err(|_| "存档的 auth.json 已损坏。".to_string())?;
    let access_token = auth
        .get("tokens")
        .and_then(|tokens| tokens.get("access_token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "账户缺少 access_token，请重新授权。".to_string())?;
    let account_id = if account_id.is_empty() {
        identity_from_auth_json(&auth).account_id
    } else {
        account_id.to_string()
    };
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client
        .get(RESET_CREDITS_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("User-Agent", "codex_cli_rs");
    if !account_id.is_empty() {
        request = request.header("ChatGPT-Account-ID", &account_id);
    }
    let response = request
        .send()
        .map_err(|error| format!("重置卡查询失败：{error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("无法读取重置卡信息：{error}"))?;
    if !status.is_success() {
        return Err(format!("重置卡查询失败：HTTP {status}"));
    }
    parse_reset_credits(&body)
}

pub(super) fn parse_reset_credits(body: &str) -> Result<ResetCredits, String> {
    let payload: ResetCreditsResponse =
        serde_json::from_str(body).map_err(|error| format!("重置卡响应格式不符合预期：{error}"))?;
    Ok(ResetCredits {
        available_count: payload.available_count,
        credits: payload
            .credits
            .into_iter()
            .map(|credit| ResetCredit {
                id: credit.id,
                title: credit.title,
                status: credit.status,
                expires_at: credit.expires_at,
                granted_at: credit.granted_at,
            })
            .collect(),
    })
}

pub(super) fn fetch_account_usage(
    auth_json: &str,
    account_id: &str,
) -> Result<AccountUsage, String> {
    let auth: Value =
        serde_json::from_str(auth_json).map_err(|_| "存档的 auth.json 已损坏。".to_string())?;
    let access_token = auth
        .get("tokens")
        .and_then(|tokens| tokens.get("access_token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "账户缺少 access_token，请重新授权。".to_string())?;
    let account_id = if account_id.is_empty() {
        identity_from_auth_json(&auth).account_id
    } else {
        account_id.to_string()
    };
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client
        .get(ACCOUNT_USAGE_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("User-Agent", "codex_cli_rs");
    if !account_id.is_empty() {
        request = request.header("ChatGPT-Account-ID", &account_id);
    }
    let response = request
        .send()
        .map_err(|error| format!("账户信息查询失败：{error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("无法读取账户信息：{error}"))?;
    if !status.is_success() {
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .or_else(|| value.get("error").and_then(|error| error.get("message")))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| body.trim().to_string());
        return Err(if message.is_empty() {
            format!("账户信息查询失败：HTTP {status}")
        } else {
            format!("账户信息查询失败：{message}")
        });
    }
    parse_account_usage(&body)
}

pub(super) fn parse_account_usage(body: &str) -> Result<AccountUsage, String> {
    let payload: Value =
        serde_json::from_str(body).map_err(|_| "账户额度响应不是有效的 JSON。".to_string())?;
    let rate_limit = payload.get("rate_limit").and_then(Value::as_object);
    let primary = rate_limit
        .and_then(|limit| limit.get("primary_window"))
        .and_then(usage_window_from_value);
    let secondary = rate_limit
        .and_then(|limit| limit.get("secondary_window"))
        .and_then(usage_window_from_value);
    let credits = payload.get("credits").and_then(Value::as_object);
    if primary.is_none()
        && secondary.is_none()
        && credits.is_none()
        && rate_limit
            .and_then(|limit| limit.get("allowed"))
            .and_then(Value::as_bool)
            .is_none()
    {
        return Err("账户额度响应缺少可识别的额度信息。".to_string());
    }
    Ok(AccountUsage {
        plan_type: payload
            .get("plan_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        primary,
        secondary,
    })
}

pub(super) fn usage_window_from_value(value: &Value) -> Option<UsageWindow> {
    let window = value.as_object()?;
    let used_percent = window.get("used_percent")?.as_f64()?;
    let window_minutes = window
        .get("limit_window_seconds")
        .and_then(Value::as_f64)
        .filter(|seconds| *seconds > 0.0)
        .map(|seconds| (seconds / 60.0).ceil() as i64);
    let resets_at = window
        .get("reset_at")
        .and_then(Value::as_f64)
        .map(|seconds| (seconds * 1000.0) as i64);
    Some(UsageWindow {
        used_percent,
        window_minutes,
        resets_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn oauth_auth(
        account_id: &str,
        user_id: &str,
        refresh_token: &str,
        access_token: &str,
    ) -> String {
        let encode = |value: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
        let claims = json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id,
                "chatgpt_user_id": user_id
            }
        });
        let id_token = format!(
            "{}.{}.{}",
            encode(br#"{"alg":"none","typ":"JWT"}"#),
            encode(claims.to_string().as_bytes()),
            encode(b"signature")
        );
        json!({
            "tokens": {
                "account_id": account_id,
                "id_token": id_token,
                "access_token": access_token,
                "refresh_token": refresh_token
            }
        })
        .to_string()
    }

    #[test]
    fn isolates_accounts_by_product() {
        let directory =
            std::env::temp_dir().join(format!("cortana-product-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        upsert_profile_from_auth(
            &state,
            &oauth_auth(
                "codex-account",
                "codex-user",
                "codex-refresh",
                "codex-access",
            ),
            "Codex",
        )
        .unwrap();
        let connection = open_database(&state).unwrap();
        connection
            .execute(
                "INSERT INTO accounts (id, product, alias, auth_json, created_at, updated_at) VALUES ('agy', 'antigravity', 'Antigravity', '{}', 1, 1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO accounts (id, product, alias, auth_json, created_at, updated_at) VALUES ('grok', 'grok', 'Grok', '{}', 1, 1)",
                [],
            )
            .unwrap();
        assert_eq!(list_profiles(&connection, None).unwrap().len(), 1);
        assert_eq!(
            list_profiles_for_product(&connection, AccountProduct::Antigravity, None)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            list_profiles_for_product(&connection, AccountProduct::Grok, None)
                .unwrap()
                .len(),
            1
        );
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn switches_without_confirmation_when_external_auth_has_no_refresh_token() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let target = upsert_profile_from_auth(
            &state,
            &oauth_auth("target-account", "target-user", "target-rt", "target-at"),
            "target",
        )
        .unwrap();
        write_auth_json_atomically(
            &directory.join("auth.json"),
            &oauth_auth(
                "external-account",
                "external-user",
                "external-rt",
                "external-at",
            ),
        )
        .unwrap();
        assert!(switch_profile_internal(&state, &target.id, false)
            .unwrap_err()
            .contains("工具外"));

        write_auth_json_atomically(&directory.join("auth.json"), r#"{"tokens":{}}"#).unwrap();

        switch_profile_internal(&state, &target.id, false).unwrap();

        let connection = open_database(&state).unwrap();
        let (status, active_id) =
            resolve_auth_state(&connection, &directory.join("auth.json")).unwrap();
        assert_eq!(status.kind, "managed");
        assert_eq!(active_id.as_deref(), Some(target.id.as_str()));
        assert_eq!(get_setting(&connection, "active_profile_id").unwrap(), None);
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn profiles_are_deduplicated_by_account_and_user_identity() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let first = upsert_profile_from_auth(
            &state,
            &oauth_auth("shared-account", "user-1", "rt-1", "at-1"),
            "first",
        )
        .unwrap();
        let updated = upsert_profile_from_auth(
            &state,
            &oauth_auth("shared-account", "user-1", "rt-2", "at-2"),
            "updated",
        )
        .unwrap();
        let other_user = upsert_profile_from_auth(
            &state,
            &oauth_auth("shared-account", "user-2", "rt-3", "at-3"),
            "other",
        )
        .unwrap();

        assert_eq!(first.id, updated.id);
        assert_ne!(first.id, other_user.id);
        assert!(update_profile_internal(
            &state,
            &other_user.id,
            "duplicate",
            &oauth_auth("shared-account", "user-1", "rt-4", "at-4"),
        )
        .unwrap_err()
        .contains("已存在"));
        assert_eq!(
            list_profiles(&open_database(&state).unwrap(), None)
                .unwrap()
                .len(),
            2
        );
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn matches_current_oauth_after_token_rotation() {
        let directory =
            std::env::temp_dir().join(format!("cortana-auth-match-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let profile = upsert_profile_from_auth(
            &state,
            &oauth_auth("same-account", "same-user", "old-rt", "old-at"),
            "OAuth",
        )
        .unwrap();
        write_auth_json_atomically(
            &directory.join("auth.json"),
            &oauth_auth("same-account", "same-user", "new-rt", "new-at"),
        )
        .unwrap();

        let (status, active_id) = resolve_auth_state(
            &open_database(&state).unwrap(),
            &directory.join("auth.json"),
        )
        .unwrap();

        assert_eq!(status.kind, "managed");
        assert_eq!(active_id.as_deref(), Some(profile.id.as_str()));
        assert!(
            get_profile_auth_json(&open_database(&state).unwrap(), &profile.id)
                .unwrap()
                .contains("new-rt")
        );
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn parses_codex_usage_windows() {
        let usage = parse_account_usage(
                r#"{
                  "plan_type":"plus",
                  "rate_limit":{
                    "allowed":true,
                    "primary_window":{"used_percent":42,"limit_window_seconds":18000,"reset_at":1777000000},
                    "secondary_window":{"used_percent":5,"limit_window_seconds":604800,"reset_at":1777600000}
                  },
                  "credits":{"has_credits":true,"unlimited":false,"balance":"9.99"}
                }"#,
            )
            .unwrap();

        assert_eq!(usage.plan_type, "plus");
        assert_eq!(usage.primary.unwrap().window_minutes, Some(300));
        assert_eq!(usage.secondary.unwrap().used_percent, 5.0);
    }
    #[test]
    fn parses_reset_credit_details() {
        let credits = parse_reset_credits(
            r#"{"credits":[{"id":"credit-1","reset_type":"codex_rate_limits","status":"available","granted_at":"2026-06-25T10:00:00Z","expires_at":"2026-07-18T00:41:14Z","redeem_started_at":null,"redeemed_at":null,"title":"Full reset"}],"available_count":1,"total_earned_count":0}"#,
        )
        .unwrap();

        assert_eq!(credits.available_count, 1);
        assert_eq!(credits.credits[0].id, "credit-1");
        assert_eq!(credits.credits[0].title, "Full reset");
        assert_eq!(credits.credits[0].status, "available");
    }
    #[test]
    fn updates_profile_alias_auth_and_active_file() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let profile = upsert_profile_from_auth(
            &state,
            &oauth_auth("edit-account", "edit-user", "old-rt", "old-at"),
            "旧名称",
        )
        .unwrap();
        switch_profile_internal(&state, &profile.id, true).unwrap();
        let updated_auth = oauth_auth("edit-account", "edit-user", "new-rt", "new-at");
        let formatted_auth =
            serde_json::to_string_pretty(&serde_json::from_str::<Value>(&updated_auth).unwrap())
                .unwrap();

        let updated =
            update_profile_internal(&state, &profile.id, "新名称", &updated_auth).unwrap();

        assert_eq!(updated.alias, "新名称");
        assert_eq!(
            fs::read_to_string(directory.join("auth.json")).unwrap(),
            formatted_auth
        );
        let connection = open_database(&state).unwrap();
        assert_eq!(
            get_profile_auth_json(&connection, &profile.id).unwrap(),
            formatted_auth
        );
        assert_eq!(
            resolve_auth_state(&connection, &directory.join("auth.json"))
                .unwrap()
                .0
                .kind,
            "managed"
        );
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn switches_between_relay_and_oauth_files_and_deduplicates_relays() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        save_codex_config_internal(&state, "# keep\nmodel = \"gpt-test\"\n").unwrap();

        let relay = upsert_relay_profile(
            &state,
            "relay-key",
            "https://relay.example.com/v1/",
            "Relay",
        )
        .unwrap();
        let duplicate = upsert_relay_profile(
            &state,
            "relay-key",
            "https://relay.example.com/v1",
            "Renamed",
        )
        .unwrap();
        let other =
            upsert_relay_profile(&state, "relay-key", "https://other.example.com/v1", "Other")
                .unwrap();
        assert_eq!(relay.id, duplicate.id);
        assert_ne!(relay.id, other.id);

        switch_profile_internal(&state, &relay.id, true).unwrap();
        let auth: Value =
            serde_json::from_str(&fs::read_to_string(directory.join("auth.json")).unwrap())
                .unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], "relay-key");
        let config = fs::read_to_string(directory.join("config.toml")).unwrap();
        assert!(config.contains("# keep"));
        assert!(config.contains("model_provider = \"relay\""));
        assert!(config.contains("[model_providers.relay]"));
        assert!(config.contains("base_url = \"https://relay.example.com/v1\""));
        write_file_atomically(
            &directory.join("config.toml"),
            &config.replace(
                "base_url = \"https://relay.example.com/v1\"",
                "base_url = \"https://relay.example.com/v1/\"",
            ),
        )
        .unwrap();
        assert_eq!(
            resolve_auth_state(
                &open_database(&state).unwrap(),
                &directory.join("auth.json"),
            )
            .unwrap()
            .0
            .kind,
            "managed"
        );

        let oauth = upsert_profile_from_auth(
            &state,
            &oauth_auth("oauth-account", "oauth-user", "oauth-rt", "oauth-at"),
            "OAuth",
        )
        .unwrap();
        switch_profile_internal(&state, &oauth.id, true).unwrap();
        let config = fs::read_to_string(directory.join("config.toml")).unwrap();
        assert!(config.contains("# keep"));
        assert!(!config.contains("model_provider"));
        assert!(!config.contains("model_providers.relay"));
        fs::remove_dir_all(directory).unwrap();
    }
}
