use super::{accounts::*, codex::write_file_atomically, db::*, *};
use rusqlite::TransactionBehavior;

const ACTIVE_PROFILE_SETTING: &str = "antigravity_active_profile_id";
const TOKEN_REFRESH_SKEW_SECONDS: i64 = 15 * 60;
const CLOUD_CODE_BASE_URL: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";
const ANTIGRAVITY_USER_AGENT: &str = "vscode/1.X.X (Antigravity/4.3.0)";
const QUOTA_ENDPOINTS: [&str; 3] = [
    "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:fetchAvailableModels",
    "https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
    "https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
];
const QUOTA_SUMMARY_ENDPOINTS: [&str; 3] = [
    "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:retrieveUserQuotaSummary",
    "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
];
static SWITCH_LOCK: Mutex<()> = Mutex::new(());

fn config_path(state: &AppState) -> PathBuf {
    state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home)
        .join(".gemini/antigravity-cli/settings.json")
}

#[tauri::command]
pub(super) fn get_antigravity_config(state: State<'_, AppState>) -> Result<ConfigFile, String> {
    read_antigravity_config(&config_path(&state))
}

fn read_antigravity_config(path: &Path) -> Result<ConfigFile, String> {
    config::read_config(path, "{}", "Antigravity settings.json")
}

#[tauri::command]
pub(super) fn validate_antigravity_config(content: String) -> Vec<ConfigDiagnostic> {
    config::validate_json_object(&content, "Antigravity settings.json")
}

#[tauri::command]
pub(super) fn format_antigravity_config(content: String) -> Result<String, String> {
    config::format_json_object(&content, "Antigravity settings.json")
}

#[tauri::command]
pub(super) fn save_antigravity_config(
    state: State<'_, AppState>,
    content: String,
) -> Result<(), String> {
    save_antigravity_config_at(&config_path(&state), &content)
}

fn save_antigravity_config_at(path: &Path, content: &str) -> Result<(), String> {
    config::parse_json_object(content, "Antigravity settings.json")?;
    write_file_atomically(path, content)
}

#[derive(Deserialize)]
struct GoogleUserInfo {
    id: Option<String>,
    email: String,
    name: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
struct AntigravityToken {
    access_token: String,
    refresh_token: String,
    token_type: String,
    expiry: String,
    #[serde(default)]
    id_token: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct AntigravityAuth {
    token: AntigravityToken,
    auth_method: String,
}

#[derive(Serialize)]
struct RuntimeToken<'a> {
    access_token: &'a str,
    token_type: &'a str,
    refresh_token: &'a str,
    expiry: &'a str,
}

#[derive(Serialize)]
struct RuntimeCredential<'a> {
    token: RuntimeToken<'a>,
    auth_method: &'a str,
}

#[derive(Deserialize)]
struct LoadCodeAssistResponse {
    #[serde(rename = "cloudaicompanionProject")]
    project_id: Option<String>,
    #[serde(rename = "currentTier")]
    current_tier: Option<AntigravityTier>,
    #[serde(rename = "paidTier")]
    paid_tier: Option<AntigravityTier>,
    #[serde(rename = "allowedTiers", default)]
    allowed_tiers: Vec<AntigravityTier>,
    #[serde(rename = "ineligibleTiers", default)]
    ineligible_tiers: Vec<Value>,
}

#[derive(Deserialize)]
struct AntigravityTier {
    id: Option<String>,
    name: Option<String>,
    #[serde(rename = "isDefault", alias = "is_default", default)]
    is_default: bool,
}

#[derive(Deserialize)]
struct AvailableModelsResponse {
    #[serde(default)]
    models: HashMap<String, AvailableModel>,
}

#[derive(Deserialize)]
struct AvailableModel {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "quotaInfo")]
    quota_info: Option<AvailableModelQuota>,
}

#[derive(Deserialize)]
struct AvailableModelQuota {
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
}

#[derive(Deserialize)]
struct QuotaSummaryResponse {
    #[serde(default)]
    groups: Vec<QuotaSummaryGroup>,
}

#[derive(Deserialize)]
struct QuotaSummaryGroup {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(default)]
    buckets: Vec<QuotaSummaryBucket>,
}

#[derive(Deserialize)]
struct QuotaSummaryBucket {
    #[serde(rename = "bucketId")]
    bucket_id: Option<String>,
    window: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
}

pub(super) fn app_status(app: &tauri::AppHandle, state: &AppState) -> Result<AppStatus, String> {
    let connection = open_database(state)?;
    let active_id = active_profile_id(&connection)?;
    let profiles = list_profiles_for_product(
        &connection,
        AccountProduct::Antigravity,
        active_id.as_deref(),
    )?;
    let managed = active_id
        .as_deref()
        .is_some_and(|id| profiles.iter().any(|profile| profile.id == id));
    Ok(AppStatus {
        profiles,
        detected_profile: None,
        auth_path: credential_location().to_string(),
        auth_state: AuthState {
            kind: if managed { "managed" } else { "missing" }.to_string(),
            message: if managed {
                "显示上次由 Cortana 切换的 Antigravity CLI 账号。"
            } else {
                "尚未通过 Cortana 切换 Antigravity CLI 账号。"
            }
            .to_string(),
        },
        autostart_enabled: app.autolaunch().is_enabled().unwrap_or(false),
        web_access: local_web::web_access_status(app, state)?,
    })
}

pub(super) fn upsert_oauth_profile(
    state: &AppState,
    token: &OAuthTokenResponse,
    requested_alias: &str,
) -> Result<ProfileSummary, String> {
    let access_token = token.access_token.as_deref().unwrap_or_default().trim();
    let refresh_token = token.refresh_token.as_deref().unwrap_or_default().trim();
    if access_token.is_empty() || refresh_token.is_empty() {
        return Err("Antigravity OAuth 未返回完整凭据。".to_string());
    }
    let user = fetch_google_user_info(access_token)?;
    let email = user.email.trim();
    if email.is_empty() {
        return Err("Google OAuth 未返回账号邮箱。".to_string());
    }
    let account_id = user
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(email);
    let auth_json = build_auth_json(token, access_token, refresh_token)?;

    let mut connection = open_database(state)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let existing_id = transaction
        .query_row(
            "SELECT id FROM accounts WHERE product = 'antigravity' AND (account_id = ?1 OR email = ?2 COLLATE NOCASE) LIMIT 1",
            params![account_id, email],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?;
    let now = now_millis();
    let id = existing_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let requested_alias = requested_alias.trim();
    let default_alias = user
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(email);
    if transaction
        .execute(
            "UPDATE accounts SET account_id = ?1, email = ?2, alias = CASE WHEN ?3 = '' THEN alias ELSE ?3 END, auth_json = ?4, updated_at = ?5 WHERE id = ?6 AND product = 'antigravity'",
            params![account_id, email, requested_alias, auth_json, now, id],
        )
        .map_err(database_error)?
        == 0
    {
        let alias = if requested_alias.is_empty() {
            default_alias
        } else {
            requested_alias
        };
        transaction
            .execute(
                "INSERT INTO accounts (id, product, account_type, account_id, email, alias, plan_type, auth_json, created_at, updated_at, sort_order) VALUES (?1, 'antigravity', 'oauth', ?2, ?3, ?4, '', ?5, ?6, ?6, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'antigravity'), 0))",
                params![id, account_id, email, alias, auth_json, now],
            )
            .map_err(database_error)?;
    }
    transaction.commit().map_err(database_error)?;
    get_profile_summary_for_product(&connection, AccountProduct::Antigravity, &id, None)
}

pub(super) fn import_current_profile(
    _state: &AppState,
    _requested_alias: Option<String>,
) -> Result<ProfileSummary, String> {
    Err("请使用浏览器 OAuth 添加 Antigravity 账号。".to_string())
}

pub(super) fn switch_profile(
    state: &AppState,
    profile_id: &str,
    _force: bool,
) -> Result<ProfileSummary, String> {
    let _guard = SWITCH_LOCK
        .try_lock()
        .map_err(|_| "已有 Antigravity 账号切换正在进行，请稍后重试。".to_string())?;
    let mut connection = open_database(state)?;
    let auth_json = connection
        .query_row(
            "SELECT auth_json FROM accounts WHERE id = ?1 AND product = 'antigravity'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    let mut auth = parse_auth(&auth_json)?;
    if refresh_if_needed(&mut auth)? {
        let refreshed = serde_json::to_string_pretty(&auth).map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE accounts SET auth_json = ?1, updated_at = ?2 WHERE id = ?3 AND product = 'antigravity'",
                params![refreshed, now_millis(), profile_id],
            )
            .map_err(database_error)?;
    }
    let runtime_credential = build_runtime_credential(&auth)?;
    write_system_credential(&runtime_credential)?;
    mark_profile_active(&mut connection, profile_id, now_millis())?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Antigravity,
        profile_id,
        Some(profile_id),
    )
}

pub(super) fn update_alias(
    state: &AppState,
    profile_id: &str,
    requested_alias: &str,
) -> Result<ProfileSummary, String> {
    let alias = requested_alias.trim();
    if alias.is_empty() {
        return Err("请输入账号别名。".to_string());
    }
    let connection = open_database(state)?;
    let changed = connection
        .execute(
            "UPDATE accounts SET alias = ?1, updated_at = ?2 WHERE id = ?3 AND product = 'antigravity'",
            params![alias, now_millis(), profile_id],
        )
        .map_err(database_error)?;
    if changed == 0 {
        return Err("账户不存在。".to_string());
    }
    let active_id = active_profile_id(&connection)?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Antigravity,
        profile_id,
        active_id.as_deref(),
    )
}

pub(super) fn refresh_profile_usage(
    state: &AppState,
    profile_id: &str,
) -> Result<ProfileSummary, String> {
    let connection = open_database(state)?;
    let (auth_json, cached_quota, current_plan) = connection
        .query_row(
            "SELECT auth_json, antigravity_quota_json, plan_type FROM accounts WHERE id = ?1 AND product = 'antigravity'",
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
        .ok_or_else(|| "账户不存在。".to_string())?;
    let cached_quota = cached_quota
        .as_deref()
        .and_then(|value| serde_json::from_str::<AntigravityQuota>(value).ok())
        .unwrap_or_default();
    let mut auth = parse_auth(&auth_json)?;
    if refresh_if_needed(&mut auth)? {
        let refreshed = serde_json::to_string_pretty(&auth).map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE accounts SET auth_json = ?1, updated_at = ?2 WHERE id = ?3 AND product = 'antigravity'",
                params![refreshed, now_millis(), profile_id],
            )
            .map_err(database_error)?;
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?;
    let (project_id, plan_type) = fetch_project_and_plan(&client, &auth.token.access_token)
        .unwrap_or((cached_quota.project_id.clone(), None));
    let project_id = project_id.or(cached_quota.project_id);
    let (models, forbidden) =
        fetch_available_models(&client, &auth.token.access_token, project_id.as_deref())?;
    let groups = if forbidden {
        Vec::new()
    } else {
        fetch_quota_summary(&client, &auth.token.access_token, project_id.as_deref())
            .unwrap_or_default()
    };
    let quota = AntigravityQuota {
        project_id,
        forbidden,
        models,
        groups,
    };
    let quota_json = serde_json::to_string(&quota).map_err(|error| error.to_string())?;
    let plan_type = plan_type.unwrap_or(current_plan);
    let now = now_millis();
    connection
        .execute(
            "UPDATE accounts SET plan_type = ?1, antigravity_quota_json = ?2, usage_updated_at = ?3 WHERE id = ?4 AND product = 'antigravity'",
            params![plan_type, quota_json, now, profile_id],
        )
        .map_err(database_error)?;
    let active_id = active_profile_id(&connection)?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Antigravity,
        profile_id,
        active_id.as_deref(),
    )
}

fn fetch_project_and_plan(
    client: &Client,
    access_token: &str,
) -> Result<(Option<String>, Option<String>), String> {
    let response = client
        .post(format!("{CLOUD_CODE_BASE_URL}/v1internal:loadCodeAssist"))
        .bearer_auth(access_token)
        .header("User-Agent", ANTIGRAVITY_USER_AGENT)
        .json(&json!({ "metadata": { "ideType": "ANTIGRAVITY" } }))
        .send()
        .map_err(|error| format!("Antigravity 套餐查询失败：{error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Antigravity 套餐查询失败：HTTP {status}"));
    }
    let payload = response
        .json::<LoadCodeAssistResponse>()
        .map_err(|error| format!("Antigravity 套餐响应格式不符合预期：{error}"))?;
    let plan_type = resolve_plan_type(&payload);
    Ok((payload.project_id, plan_type))
}

fn resolve_plan_type(payload: &LoadCodeAssistResponse) -> Option<String> {
    let tier_name = |tier: &AntigravityTier| {
        tier.name
            .as_deref()
            .or(tier.id.as_deref())
            .map(str::to_string)
    };
    if let Some(plan) = payload.paid_tier.as_ref().and_then(tier_name) {
        return Some(plan);
    }
    if payload.ineligible_tiers.is_empty() {
        return payload.current_tier.as_ref().and_then(tier_name);
    }
    payload
        .allowed_tiers
        .iter()
        .find(|tier| tier.is_default)
        .and_then(tier_name)
        .map(|plan| format!("{plan} (Restricted)"))
}

fn fetch_available_models(
    client: &Client,
    access_token: &str,
    project_id: Option<&str>,
) -> Result<(Vec<AntigravityModelQuota>, bool), String> {
    let mut last_error = None;
    for endpoint in QUOTA_ENDPOINTS {
        let mut include_project = project_id.is_some();
        loop {
            let body = if include_project {
                json!({ "project": project_id })
            } else {
                json!({})
            };
            let response = match client
                .post(endpoint)
                .bearer_auth(access_token)
                .header("User-Agent", ANTIGRAVITY_USER_AGENT)
                .json(&body)
                .send()
            {
                Ok(response) => response,
                Err(error) => {
                    last_error = Some(format!("Antigravity 额度查询失败：{error}"));
                    break;
                }
            };
            let status = response.status();
            if status == reqwest::StatusCode::FORBIDDEN {
                if include_project {
                    include_project = false;
                    continue;
                }
                return Ok((Vec::new(), true));
            }
            let response_body = response
                .text()
                .map_err(|error| format!("无法读取 Antigravity 额度响应：{error}"))?;
            if status.is_success() {
                return parse_available_models(&response_body).map(|models| (models, false));
            }
            let error = format!("Antigravity 额度查询失败：HTTP {status}");
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                last_error = Some(error);
                break;
            }
            return Err(error);
        }
    }
    Err(last_error.unwrap_or_else(|| "Antigravity 额度查询失败。".to_string()))
}

fn parse_available_models(body: &str) -> Result<Vec<AntigravityModelQuota>, String> {
    let payload: AvailableModelsResponse = serde_json::from_str(body)
        .map_err(|error| format!("Antigravity 额度响应格式不符合预期：{error}"))?;
    let mut models = payload
        .models
        .into_iter()
        .filter_map(|(model_id, model)| {
            let normalized = model_id.to_lowercase();
            let supported = ["gemini", "claude", "gpt", "image", "imagen"]
                .iter()
                .any(|prefix| normalized.starts_with(prefix));
            let quota = model.quota_info?;
            supported.then(|| AntigravityModelQuota {
                display_name: model.display_name.unwrap_or_else(|| model_id.clone()),
                model_id,
                remaining_percent: remaining_percent(quota.remaining_fraction),
                resets_at: parse_reset_time(quota.reset_time.as_deref()),
            })
        })
        .collect::<Vec<_>>();
    models.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.model_id.cmp(&right.model_id))
    });
    Ok(models)
}

fn fetch_quota_summary(
    client: &Client,
    access_token: &str,
    project_id: Option<&str>,
) -> Option<Vec<AntigravityQuotaGroup>> {
    let body = project_id
        .map(|project| json!({ "project": project }))
        .unwrap_or_else(|| json!({}));
    for endpoint in QUOTA_SUMMARY_ENDPOINTS {
        let response = client
            .post(endpoint)
            .bearer_auth(access_token)
            .header("User-Agent", ANTIGRAVITY_USER_AGENT)
            .json(&body)
            .send();
        let Ok(response) = response else { continue };
        let status = response.status();
        if status.is_success() {
            return response
                .text()
                .ok()
                .and_then(|body| parse_quota_summary(&body).ok());
        }
        if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS {
            return None;
        }
    }
    None
}

fn parse_quota_summary(body: &str) -> Result<Vec<AntigravityQuotaGroup>, String> {
    let payload: QuotaSummaryResponse = serde_json::from_str(body)
        .map_err(|error| format!("Antigravity 分组额度响应格式不符合预期：{error}"))?;
    Ok(payload
        .groups
        .into_iter()
        .map(|group| AntigravityQuotaGroup {
            display_name: group.display_name.unwrap_or_else(|| "模型额度".to_string()),
            buckets: group
                .buckets
                .into_iter()
                .map(|bucket| {
                    let window = bucket.window.unwrap_or_default();
                    AntigravityQuotaBucket {
                        bucket_id: bucket.bucket_id.unwrap_or_else(|| window.clone()),
                        display_name: bucket
                            .display_name
                            .unwrap_or_else(|| quota_window_label(&window).to_string()),
                        window,
                        remaining_percent: remaining_percent(bucket.remaining_fraction),
                        resets_at: parse_reset_time(bucket.reset_time.as_deref()),
                    }
                })
                .collect(),
        })
        .collect())
}

fn remaining_percent(fraction: Option<f64>) -> i64 {
    (fraction.unwrap_or_default() * 100.0)
        .round()
        .clamp(0.0, 100.0) as i64
}

fn parse_reset_time(value: Option<&str>) -> Option<i64> {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp_millis())
}

fn quota_window_label(window: &str) -> &str {
    match window.to_lowercase().as_str() {
        "weekly" => "周额度",
        "5h" => "5 小时额度",
        _ => "剩余额度",
    }
}

pub(super) fn clear_active_profile(
    connection: &Connection,
    profile_id: &str,
) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM settings WHERE key = ?1 AND value = ?2",
            params![ACTIVE_PROFILE_SETTING, profile_id],
        )
        .map_err(database_error)?;
    Ok(())
}

fn active_profile_id(connection: &Connection) -> Result<Option<String>, String> {
    get_setting(connection, ACTIVE_PROFILE_SETTING)
}

fn parse_auth(auth_json: &str) -> Result<AntigravityAuth, String> {
    let auth: AntigravityAuth = serde_json::from_str(auth_json)
        .map_err(|_| "Antigravity OAuth 凭据格式无效。".to_string())?;
    if auth.token.access_token.trim().is_empty() || auth.token.refresh_token.trim().is_empty() {
        return Err("Antigravity OAuth 凭据不完整，请重新授权。".to_string());
    }
    Ok(auth)
}

fn refresh_if_needed(auth: &mut AntigravityAuth) -> Result<bool, String> {
    let expires_at = chrono::DateTime::parse_from_rfc3339(&auth.token.expiry)
        .map(|value| value.timestamp())
        .unwrap_or_default();
    if !token_needs_refresh(expires_at, Utc::now().timestamp()) {
        return Ok(false);
    }
    let response = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())?
        .post(ANTIGRAVITY_OAUTH_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", auth.token.refresh_token.as_str()),
            ("client_id", ANTIGRAVITY_OAUTH_CLIENT_ID),
            ("client_secret", ANTIGRAVITY_OAUTH_CLIENT_SECRET),
        ])
        .send()
        .map_err(|error| format!("Antigravity 认证信息刷新请求失败：{error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Antigravity 认证信息刷新失败（HTTP {}），请重新授权。",
            status.as_u16()
        ));
    }
    let token = response
        .json::<OAuthTokenResponse>()
        .map_err(|_| "Antigravity 认证信息刷新响应不是有效 JSON。".to_string())?;
    let access_token = token.access_token.as_deref().unwrap_or_default().trim();
    if access_token.is_empty() {
        return Err("Antigravity 认证信息刷新未返回 access_token。".to_string());
    }
    auth.token.access_token = access_token.to_string();
    auth.token.token_type = token.token_type.unwrap_or_else(|| "Bearer".to_string());
    auth.token.expiry = (Utc::now()
        + chrono::Duration::seconds(token.expires_in.unwrap_or(3600).max(0)))
    .to_rfc3339_opts(SecondsFormat::Micros, true);
    if let Some(refresh_token) = token.refresh_token.filter(|value| !value.trim().is_empty()) {
        auth.token.refresh_token = refresh_token;
    }
    if let Some(id_token) = token.id_token.filter(|value| !value.trim().is_empty()) {
        auth.token.id_token = id_token;
    }
    Ok(true)
}

fn token_needs_refresh(expires_at: i64, now: i64) -> bool {
    expires_at <= now + TOKEN_REFRESH_SKEW_SECONDS
}

fn build_runtime_credential(auth: &AntigravityAuth) -> Result<String, String> {
    serde_json::to_string(&RuntimeCredential {
        token: RuntimeToken {
            access_token: &auth.token.access_token,
            token_type: &auth.token.token_type,
            refresh_token: &auth.token.refresh_token,
            expiry: &auth.token.expiry,
        },
        auth_method: "consumer",
    })
    .map_err(|error| error.to_string())
}

fn mark_profile_active(
    connection: &mut Connection,
    profile_id: &str,
    now: i64,
) -> Result<(), String> {
    let transaction = connection.transaction().map_err(database_error)?;
    let changed = transaction
        .execute(
            "UPDATE accounts SET last_used_at = ?1, updated_at = ?1 WHERE id = ?2 AND product = 'antigravity'",
            params![now, profile_id],
        )
        .map_err(database_error)?;
    if changed == 0 {
        return Err("账户不存在。".to_string());
    }
    transaction
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![ACTIVE_PROFILE_SETTING, profile_id],
        )
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)
}

#[cfg(target_os = "macos")]
fn write_system_credential(payload_json: &str) -> Result<(), String> {
    let value = macos_credential_value(payload_json);
    let _ = Command::new("/usr/bin/security")
        .args([
            "delete-generic-password",
            "-s",
            "gemini",
            "-a",
            "antigravity",
        ])
        .output();
    let output = Command::new("/usr/bin/security")
        .args([
            "add-generic-password",
            "-s",
            "gemini",
            "-a",
            "antigravity",
            "-w",
            &value,
            "-A",
        ])
        .output()
        .map_err(|error| format!("无法写入 macOS Keychain：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "写入 macOS Keychain 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_credential_value(payload_json: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    format!("go-keyring-base64:{}", STANDARD.encode(payload_json))
}

#[cfg(target_os = "windows")]
fn write_system_credential(payload_json: &str) -> Result<(), String> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    struct CredentialW {
        flags: u32,
        credential_type: u32,
        target_name: *const u16,
        comment: *const u16,
        last_written: FileTime,
        blob_size: u32,
        blob: *const u8,
        persist: u32,
        attribute_count: u32,
        attributes: *const std::ffi::c_void,
        target_alias: *const u16,
        user_name: *const u16,
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn CredWriteW(credential: *const CredentialW, flags: u32) -> i32;
    }

    let target = OsStr::new("gemini:antigravity")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let user = OsStr::new("antigravity")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let secret = payload_json.as_bytes();
    let credential = CredentialW {
        flags: 0,
        credential_type: 1,
        target_name: target.as_ptr(),
        comment: ptr::null(),
        last_written: FileTime { low: 0, high: 0 },
        blob_size: secret.len() as u32,
        blob: secret.as_ptr(),
        persist: 2,
        attribute_count: 0,
        attributes: ptr::null(),
        target_alias: ptr::null(),
        user_name: user.as_ptr(),
    };
    if unsafe { CredWriteW(&credential, 0) } == 0 {
        return Err(format!(
            "写入 Windows Credential Manager 失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn write_system_credential(_payload_json: &str) -> Result<(), String> {
    Err("当前系统暂不支持 Antigravity CLI 账号切换。".to_string())
}

#[cfg(target_os = "macos")]
fn credential_location() -> &'static str {
    "macOS Keychain: gemini/antigravity"
}

#[cfg(target_os = "windows")]
fn credential_location() -> &'static str {
    "Windows Credential Manager: gemini:antigravity"
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn credential_location() -> &'static str {
    "Antigravity CLI system credential"
}

fn fetch_google_user_info(access_token: &str) -> Result<GoogleUserInfo, String> {
    let response = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?
        .get(ANTIGRAVITY_OAUTH_USERINFO_URL)
        .bearer_auth(access_token)
        .send()
        .map_err(|error| format!("获取 Google 账号信息失败：{error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "获取 Google 账号信息失败（HTTP {}）。",
            status.as_u16()
        ));
    }
    response
        .json::<GoogleUserInfo>()
        .map_err(|_| "Google 账号信息响应不是有效 JSON。".to_string())
}

fn build_auth_json(
    token: &OAuthTokenResponse,
    access_token: &str,
    refresh_token: &str,
) -> Result<String, String> {
    let expires_at =
        Utc::now() + chrono::Duration::seconds(token.expires_in.unwrap_or(3600).max(0));
    serde_json::to_string_pretty(&json!({
        "token": {
            "access_token": access_token,
            "refresh_token": refresh_token,
            "token_type": token.token_type.as_deref().unwrap_or("Bearer"),
            "expiry": expires_at.to_rfc3339_opts(SecondsFormat::Micros, true),
            "id_token": token.id_token.as_deref().unwrap_or_default(),
        },
        "auth_method": "consumer",
    }))
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_formats_and_safely_saves_antigravity_cli_config() {
        let directory =
            std::env::temp_dir().join(format!("cortana-antigravity-{}", Uuid::new_v4()));
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        let path = config_path(&state);
        assert_eq!(
            path,
            directory.join(".gemini/antigravity-cli/settings.json")
        );
        assert_eq!(read_antigravity_config(&path).unwrap().content, "{}");
        write_file_atomically(&path, " \n").unwrap();
        assert_eq!(read_antigravity_config(&path).unwrap().content, "{}");

        let formatted =
            format_antigravity_config(r#"{"theme":"dark","telemetry":false}"#.into()).unwrap();
        save_antigravity_config_at(&path, &formatted).unwrap();
        assert_eq!(read_antigravity_config(&path).unwrap().content, formatted);

        assert!(save_antigravity_config_at(&path, "[]").is_err());
        assert!(save_antigravity_config_at(&path, "{").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), formatted);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stores_antigravity_oauth_in_canonical_shape() {
        let token = OAuthTokenResponse {
            access_token: Some("access".to_string()),
            refresh_token: Some("refresh".to_string()),
            id_token: Some("identity".to_string()),
            expires_in: Some(3600),
            token_type: Some("Bearer".to_string()),
        };
        let value: Value =
            serde_json::from_str(&build_auth_json(&token, "access", "refresh").unwrap()).unwrap();

        assert_eq!(value["auth_method"], "consumer");
        assert_eq!(value["token"]["refresh_token"], "refresh");
        assert_eq!(value["token"]["id_token"], "identity");
    }

    #[test]
    fn builds_official_runtime_credential_without_id_token() {
        let auth = AntigravityAuth {
            token: AntigravityToken {
                access_token: "access".to_string(),
                refresh_token: "refresh".to_string(),
                token_type: "Bearer".to_string(),
                expiry: "2026-07-17T15:22:48.323935Z".to_string(),
                id_token: "identity".to_string(),
            },
            auth_method: "consumer".to_string(),
        };
        let value: Value = serde_json::from_str(&build_runtime_credential(&auth).unwrap()).unwrap();

        assert_eq!(value["auth_method"], "consumer");
        assert_eq!(value["token"]["access_token"], "access");
        assert!(value["token"].get("id_token").is_none());
    }

    #[test]
    fn refreshes_tokens_inside_the_fifteen_minute_window() {
        assert!(!token_needs_refresh(2_000, 1_000));
        assert!(token_needs_refresh(1_900, 1_000));
    }

    #[test]
    fn parses_antigravity_plan_and_model_quotas() {
        let plan: LoadCodeAssistResponse = serde_json::from_value(json!({
            "cloudaicompanionProject": "project-1",
            "paidTier": { "name": "Google AI Pro" }
        }))
        .unwrap();
        let models = parse_available_models(
            &json!({
                "models": {
                    "gemini-3-pro": {
                        "displayName": "Gemini 3 Pro",
                        "quotaInfo": {
                            "remainingFraction": 0.42,
                            "resetTime": "2026-07-18T00:00:00Z"
                        }
                    },
                    "internal-chat": {
                        "quotaInfo": { "remainingFraction": 1.0 }
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(resolve_plan_type(&plan).as_deref(), Some("Google AI Pro"));
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].remaining_percent, 42);
        assert!(models[0].resets_at.is_some());
    }

    #[test]
    fn parses_antigravity_grouped_quotas() {
        let groups = parse_quota_summary(
            &json!({
                "groups": [{
                    "displayName": "Gemini Models",
                    "buckets": [{
                        "bucketId": "gemini-5h",
                        "window": "5h",
                        "remainingFraction": 0.75,
                        "resetTime": "2026-07-18T00:00:00Z"
                    }]
                }]
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(groups[0].display_name, "Gemini Models");
        assert_eq!(groups[0].buckets[0].display_name, "5 小时额度");
        assert_eq!(groups[0].buckets[0].remaining_percent, 75);
    }

    #[test]
    fn marks_only_the_selected_profile_active() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 CREATE TABLE accounts (
                   id TEXT PRIMARY KEY NOT NULL,
                   product TEXT NOT NULL,
                   last_used_at INTEGER,
                   updated_at INTEGER NOT NULL
                 );
                 INSERT INTO accounts (id, product, updated_at) VALUES
                   ('first', 'antigravity', 1),
                   ('second', 'antigravity', 1);",
            )
            .unwrap();

        mark_profile_active(&mut connection, "second", 42).unwrap();

        assert_eq!(
            active_profile_id(&connection).unwrap().as_deref(),
            Some("second")
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT last_used_at FROM accounts WHERE id = 'second'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            42
        );

        clear_active_profile(&connection, "second").unwrap();
        assert_eq!(active_profile_id(&connection).unwrap(), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn wraps_macos_credentials_for_go_keyring() {
        assert_eq!(macos_credential_value("{}"), "go-keyring-base64:e30=");
    }
}
