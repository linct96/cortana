use super::{codex::*, db::*, oauth::identity_from_auth_json, tray::*, *};

#[tauri::command]
pub(super) async fn get_app_status(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<AppStatus, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || app_status(&app, &state))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) fn switch_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    force: bool,
) -> Result<ProfileSummary, String> {
    let profile = switch_profile_internal(&state, &profile_id, force)?;
    refresh_tray(&app)?;
    Ok(profile)
}

#[tauri::command]
pub(super) async fn import_current_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    alias: Option<String>,
) -> Result<ProfileSummary, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
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
            let profile =
                upsert_profile_from_auth(&state, &auth_json, requested_alias.trim(), false)?;
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
) -> Result<ProfileSummary, String> {
    let profile = upsert_relay_profile(
        &state,
        &api_key,
        &api_base_url,
        alias.unwrap_or_default().trim(),
    )?;
    let profile = if activate {
        switch_profile_internal(&state, &profile.id, true)?
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
        refresh_profile_usage_internal(&state, &profile_id)
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
                "SELECT account_type, account_id, auth_json FROM accounts WHERE id = ?1",
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
                "UPDATE accounts SET reset_credits_available_count = ?1 WHERE id = ?2",
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
            "SELECT account_type, auth_json FROM accounts WHERE id = ?1",
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
    auth_json: String,
) -> Result<ProfileSummary, String> {
    let profile = update_profile_internal(&state, &profile_id, &alias, &auth_json)?;
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
) -> Result<ProfileSummary, String> {
    let profile = update_relay_profile_internal(
        &state,
        &profile_id,
        &alias,
        api_key.as_deref(),
        &api_base_url,
    )?;
    refresh_tray(&app)?;
    Ok(profile)
}

#[tauri::command]
pub(super) fn reorder_profiles(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    profile_ids: Vec<String>,
) -> Result<(), String> {
    let mut connection = open_database(&state)?;
    let mut existing_ids = list_profiles(&connection, None)?
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
                "UPDATE accounts SET sort_order = ?1 WHERE id = ?2",
                params![sort_order, profile_id],
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
) -> Result<(), String> {
    let mut connection = open_database(&state)?;
    let active_id = get_setting(&connection, "active_profile_id")?;
    let transaction = connection.transaction().map_err(database_error)?;
    let changed = transaction
        .execute("DELETE FROM accounts WHERE id = ?1", params![profile_id])
        .map_err(database_error)?;
    if changed == 0 {
        return Err("账户不存在。".to_string());
    }
    if active_id.as_deref() == Some(profile_id.as_str()) {
        transaction
            .execute("DELETE FROM settings WHERE key = 'active_profile_id'", [])
            .map_err(database_error)?;
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

pub(super) fn app_status(app: &tauri::AppHandle, state: &AppState) -> Result<AppStatus, String> {
    let connection = open_database(state)?;
    let configured_active_profile_id = get_setting(&connection, "active_profile_id")?;
    let auth_path = auth_path(state)?;
    let auth_state = resolve_auth_state(
        &connection,
        configured_active_profile_id.as_deref(),
        &auth_path,
    )?;
    let active_profile_id = managed_active_profile_id(configured_active_profile_id, &auth_state);
    let profiles = list_profiles(&connection, active_profile_id.as_deref())?;
    let detected_profile = read_auth_json(&auth_path)?
        .map(|auth_json| {
            detected_profile_from_auth(
                &connection,
                &auth_json,
                &auth_path.with_file_name("config.toml"),
            )
        })
        .transpose()?
        .flatten();
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    Ok(AppStatus {
        profiles,
        detected_profile,
        active_profile_id,
        auth_path: auth_path.display().to_string(),
        auth_state,
        autostart_enabled,
    })
}

pub(super) fn list_profiles(
    connection: &Connection,
    active_id: Option<&str>,
) -> Result<Vec<ProfileSummary>, String> {
    let mut statement = connection
        .prepare("SELECT id, account_type, api_base_url, account_id, email, alias, plan_type, usage_primary_percent, usage_primary_window_minutes, usage_primary_resets_at, usage_secondary_percent, usage_secondary_window_minutes, usage_secondary_resets_at, credits_balance, credits_unlimited, usage_updated_at, last_used_at, updated_at, reset_credits_available_count FROM accounts ORDER BY sort_order ASC, created_at ASC")
        .map_err(database_error)?;
    let rows = statement
        .query_map([], |row| profile_summary_from_row(row, active_id))
        .map_err(database_error)?;
    let profiles = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(profiles)
}

pub(super) fn managed_active_profile_id(
    active_id: Option<String>,
    auth_state: &AuthState,
) -> Option<String> {
    (auth_state.kind == "managed")
        .then_some(active_id)
        .flatten()
}

pub(super) fn profile_summary_from_row(
    row: &rusqlite::Row<'_>,
    active_id: Option<&str>,
) -> rusqlite::Result<ProfileSummary> {
    let id: String = row.get(0)?;
    let primary_percent: Option<f64> = row.get(7)?;
    let primary_window_minutes: Option<i64> = row.get(8)?;
    let primary_resets_at: Option<i64> = row.get(9)?;
    let secondary_percent: Option<f64> = row.get(10)?;
    let secondary_window_minutes: Option<i64> = row.get(11)?;
    let secondary_resets_at: Option<i64> = row.get(12)?;
    Ok(ProfileSummary {
        is_active: active_id == Some(id.as_str()),
        id,
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
        credits_balance: row.get(13)?,
        credits_unlimited: row.get(14)?,
        usage_updated_at: row.get(15)?,
        reset_credits_available_count: row.get(18)?,
        last_used_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

pub(super) fn get_profile_summary(
    connection: &Connection,
    profile_id: &str,
    active_id: Option<&str>,
) -> Result<ProfileSummary, String> {
    connection
        .query_row(
            "SELECT id, account_type, api_base_url, account_id, email, alias, plan_type, usage_primary_percent, usage_primary_window_minutes, usage_primary_resets_at, usage_secondary_percent, usage_secondary_window_minutes, usage_secondary_resets_at, credits_balance, credits_unlimited, usage_updated_at, last_used_at, updated_at, reset_credits_available_count FROM accounts WHERE id = ?1",
            params![profile_id],
            |row| profile_summary_from_row(row, active_id),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())
}

pub(super) fn detected_profile_from_auth(
    connection: &Connection,
    auth_json: &str,
    config_path: &Path,
) -> Result<Option<ProfileSummary>, String> {
    let auth: Value =
        serde_json::from_str(auth_json).map_err(|_| "auth.json 不是有效的 JSON。".to_string())?;
    if extract_api_key_from_value(&auth).is_some() {
        let (_, provider_name, api_base_url) = read_provider_config(config_path)?;
        if profile_exists_for_relay(connection, auth_json, api_base_url.as_deref())? {
            return Ok(None);
        }
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
            account_type: ACCOUNT_TYPE_RELAY.to_string(),
            api_base_url,
            account_id: String::new(),
            email: String::new(),
            alias,
            plan_type: String::new(),
            usage_primary: None,
            usage_secondary: None,
            credits_balance: None,
            credits_unlimited: false,
            usage_updated_at: None,
            reset_credits_available_count: None,
            is_active: true,
            last_used_at: None,
            updated_at: now,
        }));
    }
    let identity = identity_from_auth_json(&auth);
    let refresh_token = extract_refresh_token(auth_json).unwrap_or_default();
    if profile_exists_for_oauth(connection, &refresh_token)? {
        return Ok(None);
    }

    let now = now_millis();
    let usage = fetch_account_usage(auth_json, &identity.account_id).ok();
    let alias = identity.email.clone();
    Ok(Some(ProfileSummary {
        id: "detected".to_string(),
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
        credits_balance: usage
            .as_ref()
            .and_then(|usage| usage.credits_balance.clone()),
        credits_unlimited: usage
            .as_ref()
            .map(|usage| usage.credits_unlimited)
            .unwrap_or(false),
        usage_updated_at: usage.map(|_| now),
        reset_credits_available_count: None,
        is_active: true,
        last_used_at: None,
        updated_at: now,
    }))
}

pub(super) fn profile_exists_for_oauth(
    connection: &Connection,
    refresh_token: &str,
) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE account_type = 'oauth' AND ?1 <> '' AND trim(COALESCE(json_extract(auth_json, '$.tokens.refresh_token'), '')) = ?1)",
            params![refresh_token],
            |row| row.get(0),
        )
        .map_err(database_error)
}

pub(super) fn profile_exists_for_relay(
    connection: &Connection,
    auth_json: &str,
    api_base_url: Option<&str>,
) -> Result<bool, String> {
    let Some(api_base_url) = api_base_url else {
        return Ok(false);
    };
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE account_type = 'relay' AND auth_hash = ?1 AND api_base_url = ?2)",
            params![auth_hash(auth_json), api_base_url],
            |row| row.get(0),
        )
        .map_err(database_error)
}

pub(super) fn resolve_auth_state(
    connection: &Connection,
    active_id: Option<&str>,
    path: &Path,
) -> Result<AuthState, String> {
    let Some(auth_json) = read_auth_json(path)? else {
        return Ok(AuthState {
            kind: "missing".to_string(),
            message: "尚未检测到 Codex 登录文件。".to_string(),
        });
    };
    let Some(active_id) = active_id else {
        return Ok(AuthState {
            kind: "unmanaged".to_string(),
            message: "当前 Codex 登录状态尚未纳入本应用管理。".to_string(),
        });
    };
    let stored = connection
        .query_row(
            "SELECT auth_hash, account_type, api_base_url FROM accounts WHERE id = ?1",
            params![active_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?;
    match stored {
        Some((hash, account_type, api_base_url))
            if hash == auth_hash(&auth_json)
                && (account_type != ACCOUNT_TYPE_RELAY
                    || relay_config_matches(
                        &path.with_file_name("config.toml"),
                        api_base_url.as_deref(),
                    )?) =>
        {
            Ok(AuthState {
                kind: "managed".to_string(),
                message: "当前 Codex 登录状态与活动账户一致。".to_string(),
            })
        }
        Some(_) => Ok(AuthState {
            kind: "external".to_string(),
            message: "当前 Codex 登录或 API 配置已在本应用之外发生变更。".to_string(),
        }),
        None => Ok(AuthState {
            kind: "unmanaged".to_string(),
            message: "活动账户已不存在，请重新导入当前登录状态。".to_string(),
        }),
    }
}

pub(super) fn switch_profile_internal(
    state: &AppState,
    profile_id: &str,
    force: bool,
) -> Result<ProfileSummary, String> {
    let mut connection = open_database(state)?;
    let active_id = get_setting(&connection, "active_profile_id")?;
    let path = auth_path(state)?;
    let status = resolve_auth_state(&connection, active_id.as_deref(), &path)?;
    let external_auth_has_credential =
        read_auth_json(&path)?.is_some_and(|auth_json| has_usable_credential(&auth_json));
    if status.kind == "external" && external_auth_has_credential && !force {
        return Err(
            "检测到工具外的 Codex 登录或 API 配置变更。请先导入当前状态，或确认后强制切换。"
                .to_string(),
        );
    }
    let row = connection
        .query_row(
            "SELECT id, account_type, api_base_url, auth_json FROM accounts WHERE id = ?1",
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
                "INSERT INTO settings (key, value) VALUES ('active_profile_id', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![row.0],
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "UPDATE accounts SET last_used_at = ?1, updated_at = ?1 WHERE id = ?2",
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
    let mut connection = open_database(state)?;
    let (existing_email, account_type) = connection
        .query_row(
            "SELECT email, account_type FROM accounts WHERE id = ?1",
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
    let alias = match requested_alias.trim() {
        "" if identity.email.is_empty() => "导入的账户".to_string(),
        "" => identity.email.clone(),
        alias => alias.to_string(),
    };
    let hash = auth_hash(&formatted_auth_json);
    let active_id = get_setting(&connection, "active_profile_id")?;
    let active = active_id.as_deref() == Some(profile_id);
    let backup = active
        .then(|| apply_profile_files(state, &formatted_auth_json, ACCOUNT_TYPE_OAUTH, None))
        .transpose()?;
    let database_result = (|| {
        let transaction = connection.transaction().map_err(database_error)?;
        let changed = transaction
            .execute(
                "UPDATE accounts SET account_id = ?1, email = ?2, alias = ?3, plan_type = CASE WHEN ?4 = '' THEN plan_type ELSE ?4 END, auth_json = ?5, auth_hash = ?6, updated_at = ?7 WHERE id = ?8",
                params![identity.account_id, identity.email, alias, identity.plan_type, formatted_auth_json, hash, now_millis(), profile_id],
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
    get_profile_summary(&connection, profile_id, active_id.as_deref())
}

pub(super) fn upsert_profile_from_auth(
    state: &AppState,
    auth_json: &str,
    requested_alias: &str,
    make_active: bool,
) -> Result<ProfileSummary, String> {
    let parsed: Value =
        serde_json::from_str(auth_json).map_err(|_| "auth.json 不是有效的 JSON。".to_string())?;
    if !parsed.is_object() {
        return Err("auth.json 必须是一个 JSON 对象。".to_string());
    }
    let refresh_token = extract_refresh_token(auth_json)?;
    let identity = identity_from_auth_json(&parsed);
    let hash = auth_hash(auth_json);
    let connection = open_database(state)?;
    let existing = connection
        .query_row(
            "SELECT id, alias FROM accounts WHERE account_type = 'oauth' AND trim(COALESCE(json_extract(auth_json, '$.tokens.refresh_token'), '')) = ?1 LIMIT 1",
            params![refresh_token],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    let now = now_millis();
    let id = if let Some((id, existing_alias)) = existing {
        let alias = if requested_alias.is_empty() {
            existing_alias
        } else {
            requested_alias.to_string()
        };
        connection
            .execute(
                "UPDATE accounts SET account_type = 'oauth', api_base_url = NULL, account_id = ?1, email = ?2, alias = ?3, plan_type = CASE WHEN ?4 = '' THEN plan_type ELSE ?4 END, auth_json = ?5, auth_hash = ?6, updated_at = ?7 WHERE id = ?8",
                params![identity.account_id, identity.email, alias, identity.plan_type, auth_json, hash, now, id],
            )
            .map_err(database_error)?;
        id
    } else {
        let id = Uuid::new_v4().to_string();
        let alias = if requested_alias.is_empty() {
            if identity.email.is_empty() {
                "导入的账户".to_string()
            } else {
                identity.email.clone()
            }
        } else {
            requested_alias.to_string()
        };
        connection
            .execute(
                "INSERT INTO accounts (id, account_type, api_base_url, account_id, email, alias, plan_type, auth_json, auth_hash, created_at, updated_at, last_used_at, sort_order) VALUES (?1, 'oauth', NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, NULL, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts), 0))",
                params![id, identity.account_id, identity.email, alias, identity.plan_type, auth_json, hash, now],
            )
            .map_err(database_error)?;
        id
    };
    if make_active {
        set_setting(&connection, "active_profile_id", &id)?;
    }
    get_profile_summary(
        &connection,
        &id,
        if make_active { Some(id.as_str()) } else { None },
    )
}

pub(super) fn upsert_relay_profile(
    state: &AppState,
    api_key: &str,
    api_base_url: &str,
    requested_alias: &str,
) -> Result<ProfileSummary, String> {
    let api_base_url = normalize_api_base_url(api_base_url)?;
    let auth_json = build_relay_auth_json(api_key)?;
    let hash = auth_hash(&auth_json);
    let connection = open_database(state)?;
    let existing = connection
        .query_row(
            "SELECT id, alias FROM accounts WHERE account_type = 'relay' AND api_base_url = ?1 AND auth_hash = ?2 LIMIT 1",
            params![api_base_url, hash],
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
                "UPDATE accounts SET alias = ?1, auth_json = ?2, auth_hash = ?3, api_base_url = ?4, updated_at = ?5 WHERE id = ?6",
                params![alias, auth_json, hash, api_base_url, now, id],
            )
            .map_err(database_error)?;
        id
    } else {
        let id = Uuid::new_v4().to_string();
        let alias = relay_alias(requested_alias, &api_base_url);
        connection
            .execute(
                "INSERT INTO accounts (id, account_type, api_base_url, account_id, email, alias, plan_type, auth_json, auth_hash, created_at, updated_at, last_used_at, sort_order) VALUES (?1, 'relay', ?2, '', '', ?3, '', ?4, ?5, ?6, ?6, NULL, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts), 0))",
                params![id, api_base_url, alias, auth_json, hash, now],
            )
            .map_err(database_error)?;
        id
    };
    let active_id = get_setting(&connection, "active_profile_id")?;
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
            "SELECT account_type, auth_json FROM accounts WHERE id = ?1",
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
    let hash = auth_hash(&auth_json);
    let duplicate_exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE account_type = 'relay' AND id <> ?1 AND api_base_url = ?2 AND auth_hash = ?3)",
            params![profile_id, api_base_url, hash],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if duplicate_exists {
        return Err("已存在使用相同 API Key 和地址的中转站账户。".to_string());
    }
    let alias = relay_alias(requested_alias, &api_base_url);
    let active_id = get_setting(&connection, "active_profile_id")?;
    let active = active_id.as_deref() == Some(profile_id);
    let backup = active
        .then(|| apply_profile_files(state, &auth_json, ACCOUNT_TYPE_RELAY, Some(&api_base_url)))
        .transpose()?;
    let database_result = (|| {
        let transaction = connection.transaction().map_err(database_error)?;
        transaction
            .execute(
                "UPDATE accounts SET alias = ?1, api_base_url = ?2, auth_json = ?3, auth_hash = ?4, updated_at = ?5 WHERE id = ?6",
                params![alias, api_base_url, auth_json, hash, now_millis(), profile_id],
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
    pub(super) credits_balance: Option<String>,
    pub(super) credits_unlimited: bool,
}

pub(super) fn refresh_profile_usage_internal(
    state: &AppState,
    profile_id: &str,
) -> Result<ProfileSummary, String> {
    let connection = open_database(state)?;
    let (account_type, account_id, auth_json) = connection
        .query_row(
            "SELECT account_type, account_id, auth_json FROM accounts WHERE id = ?1",
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
            "UPDATE accounts SET plan_type = CASE WHEN ?1 = '' THEN plan_type ELSE ?1 END, usage_primary_percent = ?2, usage_primary_window_minutes = ?3, usage_primary_resets_at = ?4, usage_secondary_percent = ?5, usage_secondary_window_minutes = ?6, usage_secondary_resets_at = ?7, credits_balance = ?8, credits_unlimited = ?9, usage_updated_at = ?10, reset_credits_available_count = ?11 WHERE id = ?12",
            params![
                usage.plan_type,
                usage.primary.as_ref().map(|window| window.used_percent),
                usage.primary.as_ref().and_then(|window| window.window_minutes),
                usage.primary.as_ref().and_then(|window| window.resets_at),
                usage.secondary.as_ref().map(|window| window.used_percent),
                usage.secondary.as_ref().and_then(|window| window.window_minutes),
                usage.secondary.as_ref().and_then(|window| window.resets_at),
                usage.credits_balance,
                usage.credits_unlimited,
                now,
                reset_credits.as_ref().map(|credits| credits.available_count),
                profile_id,
            ],
        )
        .map_err(database_error)?;
    let active_id = get_setting(&connection, "active_profile_id")?;
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
        credits_balance: credits
            .and_then(|credits| credits.get("balance"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|balance| !balance.is_empty())
            .map(str::to_string),
        credits_unlimited: credits
            .and_then(|credits| credits.get("unlimited"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
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

    #[test]
    fn only_marks_a_profile_active_for_a_managed_auth_file() {
        let active_id = Some("profile-1".to_string());
        let managed = AuthState {
            kind: "managed".to_string(),
            message: String::new(),
        };
        let missing = AuthState {
            kind: "missing".to_string(),
            message: String::new(),
        };

        assert_eq!(
            managed_active_profile_id(active_id.clone(), &managed),
            active_id
        );
        assert_eq!(
            managed_active_profile_id(Some("profile-1".to_string()), &missing),
            None
        );
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
        upsert_profile_from_auth(
            &state,
            r#"{"tokens":{"refresh_token":"current-rt"}}"#,
            "current",
            true,
        )
        .unwrap();
        let target = upsert_profile_from_auth(
            &state,
            r#"{"tokens":{"refresh_token":"target-rt"}}"#,
            "target",
            false,
        )
        .unwrap();
        write_auth_json_atomically(
            &directory.join("auth.json"),
            r#"{"tokens":{"refresh_token":"external-rt"}}"#,
        )
        .unwrap();
        assert!(switch_profile_internal(&state, &target.id, false)
            .unwrap_err()
            .contains("工具外"));

        write_auth_json_atomically(&directory.join("auth.json"), r#"{"tokens":{}}"#).unwrap();

        switch_profile_internal(&state, &target.id, false).unwrap();

        assert_eq!(
            get_setting(&open_database(&state).unwrap(), "active_profile_id").unwrap(),
            Some(target.id)
        );
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn profiles_are_deduplicated_by_refresh_token_not_account_id() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let auth = |refresh_token: &str| {
            json!({
                "tokens": {
                    "account_id": "shared-account-id",
                    "refresh_token": refresh_token
                }
            })
            .to_string()
        };

        let first = upsert_profile_from_auth(&state, &auth("rt-1"), "first", false).unwrap();
        let second = upsert_profile_from_auth(&state, &auth("rt-2"), "second", false).unwrap();
        let updated = upsert_profile_from_auth(&state, &auth("rt-1"), "updated", false).unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(first.id, updated.id);
        assert_eq!(
            list_profiles(&open_database(&state).unwrap(), None)
                .unwrap()
                .len(),
            2
        );
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn profile_order_does_not_change_with_last_used_time() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let auth = |refresh_token: &str| {
            json!({ "tokens": { "refresh_token": refresh_token } }).to_string()
        };
        let first = upsert_profile_from_auth(&state, &auth("rt-1"), "first", false).unwrap();
        let second = upsert_profile_from_auth(&state, &auth("rt-2"), "second", false).unwrap();
        let connection = open_database(&state).unwrap();
        connection
            .execute(
                "UPDATE accounts SET last_used_at = ?1 WHERE id = ?2",
                params![now_millis(), second.id],
            )
            .unwrap();

        let profiles = list_profiles(&connection, None).unwrap();

        assert_eq!(profiles[0].id, first.id);
        assert_eq!(profiles[1].id, second.id);
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn detects_whether_current_auth_is_already_saved() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        upsert_profile_from_auth(
            &state,
            r#"{"tokens":{"account_id":"account-123","refresh_token":"rt-1"}}"#,
            "saved",
            false,
        )
        .unwrap();
        let connection = open_database(&state).unwrap();

        assert!(profile_exists_for_oauth(&connection, "rt-1").unwrap());
        assert!(!profile_exists_for_oauth(&connection, "rt-2").unwrap());
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn parses_codex_usage_windows_and_credits() {
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
        assert_eq!(usage.credits_balance.as_deref(), Some("9.99"));
        assert!(!usage.credits_unlimited);
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
            r#"{"tokens":{"refresh_token":"old"}}"#,
            "旧名称",
            true,
        )
        .unwrap();
        let updated_auth = r#"{ "tokens": { "refresh_token": "new" } }"#;
        let formatted_auth = r#"{
  "tokens": {
    "refresh_token": "new"
  }
}"#;

        let updated = update_profile_internal(&state, &profile.id, "新名称", updated_auth).unwrap();

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
            resolve_auth_state(&connection, Some(&profile.id), &directory.join("auth.json"))
                .unwrap()
                .kind,
            "managed"
        );
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn detects_custom_provider_name_and_url_for_api_key_login() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-provider-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let config_path = directory.join("config.toml");
        fs::write(
                &config_path,
                "model_provider = \"custom\"\n[model_providers.custom]\nname = \"My Relay\"\nbase_url = \"https://relay.example.com/v1\"\n",
            )
            .unwrap();
        let connection = open_database(&state).unwrap();
        let profile = detected_profile_from_auth(
            &connection,
            r#"{"OPENAI_API_KEY":"relay-key"}"#,
            &config_path,
        )
        .unwrap()
        .unwrap();
        assert_eq!(profile.alias, "My Relay");
        assert_eq!(
            profile.api_base_url.as_deref(),
            Some("https://relay.example.com/v1")
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
        assert_eq!(
            resolve_auth_state(
                &open_database(&state).unwrap(),
                Some(&relay.id),
                &directory.join("auth.json"),
            )
            .unwrap()
            .kind,
            "managed"
        );

        let oauth = upsert_profile_from_auth(
            &state,
            r#"{"tokens":{"refresh_token":"oauth-rt"}}"#,
            "OAuth",
            false,
        )
        .unwrap();
        switch_profile_internal(&state, &oauth.id, true).unwrap();
        let config = fs::read_to_string(directory.join("config.toml")).unwrap();
        assert!(config.contains("# keep"));
        assert!(!config.contains("model_provider"));
        assert!(!config.contains("model_providers.relay"));
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn detects_external_relay_config_changes_and_updates_active_relay() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let relay =
            upsert_relay_profile(&state, "relay-key", "https://relay.example.com/v1", "Relay")
                .unwrap();
        switch_profile_internal(&state, &relay.id, true).unwrap();
        save_codex_config_internal(
                &state,
                "model_provider = \"relay\"\n[model_providers.relay]\nname = \"Relay\"\nbase_url = \"https://changed.example.com/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n",
            )
            .unwrap();
        let connection = open_database(&state).unwrap();
        assert_eq!(
            resolve_auth_state(&connection, Some(&relay.id), &directory.join("auth.json"),)
                .unwrap()
                .kind,
            "external"
        );
        drop(connection);

        let updated = update_relay_profile_internal(
            &state,
            &relay.id,
            "Updated",
            None,
            "http://relay.local/v1/",
        )
        .unwrap();
        assert_eq!(
            updated.api_base_url.as_deref(),
            Some("http://relay.local/v1")
        );
        let auth = fs::read_to_string(directory.join("auth.json")).unwrap();
        assert_eq!(
            extract_api_key(&auth).unwrap().as_deref(),
            Some("relay-key")
        );
        let config = fs::read_to_string(directory.join("config.toml")).unwrap();
        assert!(config.contains("base_url = \"http://relay.local/v1\""));
        assert!(refresh_profile_usage_internal(&state, &relay.id)
            .unwrap_err()
            .contains("不支持额度查询"));
        fs::remove_dir_all(directory).unwrap();
    }
}
