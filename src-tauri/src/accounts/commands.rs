use super::{codex_auth::*, profiles::*, usage::*, *};

#[tauri::command]
pub(crate) async fn get_app_status(
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
pub(crate) async fn switch_profile(
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
pub(crate) async fn open_codex_cli_with_profile(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        open_codex_cli_with_profile_internal(&state, &profile_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn import_current_profile(
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
            Ok(refresh_codex_profile_usage_guarded(&state, &profile.id)
                .map(|result| result.profile)
                .unwrap_or(profile))
        } else {
            Ok(profile)
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn add_relay_profile(
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
pub(crate) async fn refresh_profile_usage(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<UsageRefreshResult, String> {
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
            "codex" => refresh_codex_profile_usage_guarded(&state, &profile_id),
            "claude" => claude::refresh_profile_usage(&state, &profile_id).map(|profile| {
                UsageRefreshResult {
                    profile,
                    refreshed: true,
                }
            }),
            "antigravity" => {
                antigravity::refresh_profile_usage(&state, &profile_id).map(|profile| {
                    UsageRefreshResult {
                        profile,
                        refreshed: true,
                    }
                })
            }
            "grok" => {
                grok::refresh_profile_usage(&state, &profile_id).map(|profile| UsageRefreshResult {
                    profile,
                    refreshed: true,
                })
            }
            _ => Err("不支持该账户类型。".to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn get_profile_auth(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<String, String> {
    let connection = open_database(&state)?;
    get_profile_auth_json(&connection, &profile_id)
}

#[tauri::command]
pub(crate) fn update_profile(
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
pub(crate) fn update_relay_profile(
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
pub(crate) fn reorder_profiles(
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
                params![sort_order as i64, profile_id, product.as_str()],
            )
            .map_err(database_error)?;
    }
    transaction.commit().map_err(database_error)?;
    refresh_tray(&app)
}

#[tauri::command]
pub(crate) fn delete_profile(
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
pub(crate) fn set_codex_home(
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

pub(crate) fn get_active_product(state: State<'_, AppState>) -> Result<AccountProduct, String> {
    active_product(state.inner())
}

pub(super) fn active_product(state: &AppState) -> Result<AccountProduct, String> {
    let connection = open_database(state)?;
    Ok(
        match get_setting(&connection, "active_product")?.as_deref() {
            Some("claude") => AccountProduct::Claude,
            Some("antigravity") => AccountProduct::Antigravity,
            Some("grok") => AccountProduct::Grok,
            _ => AccountProduct::Codex,
        },
    )
}

pub(crate) fn get_usage_refresh_settings(
    state: State<'_, AppState>,
) -> Result<UsageRefreshSettings, String> {
    usage_refresh_settings(&state)
}

pub(crate) fn set_account_usage_refresh_settings(
    state: State<'_, AppState>,
    enabled: bool,
    active_interval_minutes: u64,
    inactive_interval_minutes: u64,
) -> Result<UsageRefreshSettings, String> {
    if !ACTIVE_REFRESH_MINUTES.contains(&active_interval_minutes) {
        return Err("启用账号刷新间隔无效。".to_string());
    }
    if !INACTIVE_REFRESH_MINUTES.contains(&inactive_interval_minutes) {
        return Err("未启用账号刷新间隔无效。".to_string());
    }
    let mut connection = open_database(&state)?;
    db::set_usage_refresh_settings(
        &mut connection,
        enabled,
        active_interval_minutes,
        inactive_interval_minutes,
    )?;
    Ok(UsageRefreshSettings {
        enabled,
        active_interval_minutes,
        inactive_interval_minutes,
    })
}

#[tauri::command]
pub(crate) async fn refresh_due_profile_usage(
    state: State<'_, AppState>,
    immediate: bool,
) -> Result<UsageRefreshRunResult, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        refresh_due_profile_usage_internal(&state, immediate)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn set_active_product(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    product: AccountProduct,
) -> Result<(), String> {
    let connection = open_database(&state)?;
    set_setting(&connection, "active_product", product.as_str())?;
    refresh_tray(&app)
}
