use super::{
    auth::{
        authentication_invalidated, backend_error_message, with_codex_auth_retry, CodexApiError,
    },
    config::auth_path,
    credits::fetch_reset_credits,
};
use crate::{
    features::accounts::{
        active_product, get_profile_summary, oauth::identity_from_auth_json, resolve_auth_state,
    },
    platform::{
        db::{database_error, get_setting, open_database},
        state::{
            now_millis, AccountProduct, AppState, ProfileSummary, ResetCredits, UsageRefreshResult,
            UsageRefreshRunResult, UsageRefreshSettings, UsageWindow, ACCOUNT_TYPE_RELAY,
            ACCOUNT_USAGE_URL,
        },
    },
};
use reqwest::blocking::Client;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::{thread, time::Duration};

const USAGE_REFRESH_TICK: Duration = Duration::from_secs(10);
const USAGE_REFRESH_MIN_INTERVAL_MILLIS: i64 = 10_000;
const DEFAULT_ACTIVE_REFRESH_MINUTES: u64 = 5;
const DEFAULT_INACTIVE_REFRESH_MINUTES: u64 = 30;
pub(crate) const ACTIVE_REFRESH_MINUTES: [u64; 4] = [1, 2, 5, 10];
pub(crate) const INACTIVE_REFRESH_MINUTES: [u64; 4] = [5, 10, 30, 60];

#[derive(Debug)]
pub(crate) struct AccountUsage {
    pub(crate) plan_type: String,
    pub(crate) primary: Option<UsageWindow>,
    pub(crate) secondary: Option<UsageWindow>,
}

pub(crate) fn usage_refresh_settings(state: &AppState) -> Result<UsageRefreshSettings, String> {
    let connection = open_database(state)?;
    let enabled = get_setting(&connection, "usage_refresh_enabled")?
        .and_then(|value| value.parse().ok())
        .unwrap_or(true);
    let active_interval_minutes = refresh_interval_setting(
        &connection,
        "usage_refresh_active_interval_minutes",
        &ACTIVE_REFRESH_MINUTES,
        DEFAULT_ACTIVE_REFRESH_MINUTES,
    )?;
    let inactive_interval_minutes = refresh_interval_setting(
        &connection,
        "usage_refresh_inactive_interval_minutes",
        &INACTIVE_REFRESH_MINUTES,
        DEFAULT_INACTIVE_REFRESH_MINUTES,
    )?;
    Ok(UsageRefreshSettings {
        enabled,
        active_interval_minutes,
        inactive_interval_minutes,
    })
}

pub(crate) fn refresh_interval_setting(
    connection: &Connection,
    key: &str,
    allowed: &[u64],
    default: u64,
) -> Result<u64, String> {
    Ok(get_setting(connection, key)?
        .and_then(|value| value.parse().ok())
        .filter(|value| allowed.contains(value))
        .unwrap_or(default))
}

pub(crate) fn refresh_due_profile_usage_internal(
    state: &AppState,
    immediate: bool,
) -> Result<UsageRefreshRunResult, String> {
    let settings = usage_refresh_settings(state)?;
    if !settings.enabled || active_product(state)? != AccountProduct::Codex {
        return Ok(UsageRefreshRunResult::default());
    }

    let profile_ids = due_codex_profile_ids(state, settings, immediate)?;
    let mut result = UsageRefreshRunResult::default();
    for profile_id in profile_ids {
        match refresh_codex_profile_usage_guarded(state, &profile_id) {
            Ok(refresh) if refresh.refreshed => result.refreshed_count += 1,
            Ok(_) => result.skipped_count += 1,
            Err(error) => {
                result.failed_count += 1;
                eprintln!("Unable to refresh Codex account {profile_id}: {error}");
            }
        }
    }
    Ok(result)
}

pub(crate) fn start_usage_refresh_scheduler(state: AppState) {
    thread::spawn(move || {
        if let Err(error) = refresh_due_profile_usage_internal(&state, true) {
            eprintln!("Unable to refresh account usage: {error}");
        }
        loop {
            thread::sleep(USAGE_REFRESH_TICK);
            if let Err(error) = refresh_due_profile_usage_internal(&state, false) {
                eprintln!("Unable to refresh account usage: {error}");
            }
        }
    });
}

pub(crate) fn due_codex_profile_ids(
    state: &AppState,
    settings: UsageRefreshSettings,
    immediate: bool,
) -> Result<Vec<String>, String> {
    let connection = open_database(state)?;
    let (_, active_id) = resolve_auth_state(&connection, &auth_path(state)?)?;
    let now = now_millis();
    let rows = connection
        .prepare(
            "SELECT id, MAX(COALESCE(usage_refresh_attempted_at, 0), COALESCE(usage_updated_at, 0))
             FROM accounts
             WHERE product = 'codex' AND account_type = 'oauth' AND oauth_invalidated_at IS NULL",
        )
        .map_err(database_error)?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;

    Ok(rows
        .into_iter()
        .filter_map(|(profile_id, last_refresh)| {
            usage_refresh_due(
                now,
                last_refresh,
                active_id.as_deref() == Some(profile_id.as_str()),
                settings,
                immediate,
            )
            .then_some(profile_id)
        })
        .collect())
}

pub(crate) fn usage_refresh_due(
    now: i64,
    last_refresh: i64,
    active: bool,
    settings: UsageRefreshSettings,
    immediate: bool,
) -> bool {
    let interval_millis = if immediate {
        USAGE_REFRESH_MIN_INTERVAL_MILLIS
    } else if active {
        settings.active_interval_minutes as i64 * 60_000
    } else {
        settings.inactive_interval_minutes as i64 * 60_000
    };
    now.saturating_sub(last_refresh) >= interval_millis
}

pub(crate) fn claim_codex_usage_refresh(
    connection: &Connection,
    profile_id: &str,
    now: i64,
) -> Result<bool, String> {
    connection
        .execute(
            "UPDATE accounts
             SET usage_refresh_attempted_at = ?1
             WHERE id = ?2
               AND product = 'codex'
               AND account_type = 'oauth'
               AND MAX(COALESCE(usage_refresh_attempted_at, 0), COALESCE(usage_updated_at, 0)) <= ?3",
            params![
                now,
                profile_id,
                now.saturating_sub(USAGE_REFRESH_MIN_INTERVAL_MILLIS)
            ],
        )
        .map(|changed| changed == 1)
        .map_err(database_error)
}

pub(crate) fn refresh_codex_profile_usage_guarded(
    state: &AppState,
    profile_id: &str,
) -> Result<UsageRefreshResult, String> {
    let connection = open_database(state)?;
    let now = now_millis();
    if !claim_codex_usage_refresh(&connection, profile_id, now)? {
        let account_type = connection
            .query_row(
                "SELECT account_type FROM accounts WHERE id = ?1 AND product = 'codex'",
                params![profile_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| "账户不存在。".to_string())?;
        if account_type == ACCOUNT_TYPE_RELAY {
            return Err("中转站账户不支持额度查询。".to_string());
        }
        let (_, active_id) = resolve_auth_state(&connection, &auth_path(state)?)?;
        return Ok(UsageRefreshResult {
            profile: get_profile_summary(&connection, profile_id, active_id.as_deref())?,
            refreshed: false,
        });
    }
    drop(connection);

    Ok(UsageRefreshResult {
        profile: refresh_profile_usage_internal(state, profile_id)?,
        refreshed: true,
    })
}

pub(crate) fn refresh_profile_usage_internal(
    state: &AppState,
    profile_id: &str,
) -> Result<ProfileSummary, String> {
    refresh_profile_usage_with_credits_internal(state, profile_id).map(|(profile, _)| profile)
}

pub(crate) fn refresh_profile_usage_with_credits_internal(
    state: &AppState,
    profile_id: &str,
) -> Result<(ProfileSummary, Option<ResetCredits>), String> {
    let usage = with_codex_auth_retry(state, profile_id, |auth| {
        fetch_account_usage(&auth.auth_json, &auth.account_id)
    })?;
    let plan_type = usage.plan_type.trim().to_lowercase();
    let reset_credits = (!plan_type.is_empty() && plan_type != "free")
        .then(|| {
            with_codex_auth_retry(state, profile_id, |auth| {
                fetch_reset_credits(&auth.auth_json, &auth.account_id)
            })
        })
        .transpose()?;
    let connection = open_database(state)?;
    let now = now_millis();
    connection
        .execute(
            "UPDATE accounts SET plan_type = CASE WHEN ?1 = '' THEN plan_type ELSE ?1 END, usage_primary_percent = ?2, usage_primary_window_minutes = ?3, usage_primary_resets_at = ?4, usage_secondary_percent = ?5, usage_secondary_window_minutes = ?6, usage_secondary_resets_at = ?7, usage_updated_at = ?8, reset_credits_available_count = ?9, oauth_invalidated_at = NULL WHERE id = ?10 AND product = 'codex'",
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
    Ok((
        get_profile_summary(&connection, profile_id, active_id.as_deref())?,
        reset_credits,
    ))
}

pub(crate) fn fetch_account_usage(
    auth_json: &str,
    account_id: &str,
) -> Result<AccountUsage, CodexApiError> {
    let auth: Value = serde_json::from_str(auth_json).map_err(|_| CodexApiError {
        message: "存档的 auth.json 已损坏。".to_string(),
        unauthorized: false,
        authentication_invalidated: false,
    })?;
    let access_token = auth
        .get("tokens")
        .and_then(|tokens| tokens.get("access_token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| CodexApiError {
            message: "账户缺少 access_token，请重新授权。".to_string(),
            unauthorized: false,
            authentication_invalidated: true,
        })?;
    let account_id = if account_id.is_empty() {
        identity_from_auth_json(&auth).account_id
    } else {
        account_id.to_string()
    };
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| CodexApiError {
            message: error.to_string(),
            unauthorized: false,
            authentication_invalidated: false,
        })?;
    let mut request = client
        .get(ACCOUNT_USAGE_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("User-Agent", "codex_cli_rs");
    if !account_id.is_empty() {
        request = request.header("ChatGPT-Account-ID", &account_id);
    }
    let response = request.send().map_err(|error| CodexApiError {
        message: format!("账户信息查询失败：{error}"),
        unauthorized: false,
        authentication_invalidated: false,
    })?;
    let status = response.status();
    let body = response.text().map_err(|error| CodexApiError {
        message: format!("无法读取账户信息：{error}"),
        unauthorized: false,
        authentication_invalidated: false,
    })?;
    if !status.is_success() {
        let message = backend_error_message(&body);
        let authentication_invalidated = authentication_invalidated(status.as_u16(), &body);
        return Err(CodexApiError {
            message: if message.is_empty() {
                format!("账户信息查询失败：HTTP {status}")
            } else {
                format!("账户信息查询失败：{message}")
            },
            unauthorized: status.as_u16() == 401,
            authentication_invalidated,
        });
    }
    parse_account_usage(&body).map_err(|message| CodexApiError {
        message,
        unauthorized: false,
        authentication_invalidated: false,
    })
}

pub(crate) fn parse_account_usage(body: &str) -> Result<AccountUsage, String> {
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

pub(crate) fn usage_window_from_value(value: &Value) -> Option<UsageWindow> {
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
