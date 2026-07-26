use super::{accounts::*, codex::write_file_atomically, db::*, grok_oauth, local_web, *};

const BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
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
    monthly_limit: Option<Cent>,
    used: Option<Cent>,
    billing_period_start: Option<String>,
    billing_period_end: Option<String>,
}

#[derive(Deserialize)]
struct Cent {
    #[serde(default)]
    val: i64,
}

#[derive(Deserialize)]
struct BillingPeriod {
    start: Option<String>,
    end: Option<String>,
}

pub(super) fn app_status(app: &tauri::AppHandle, state: &AppState) -> Result<AppStatus, String> {
    let connection = open_database(state)?;
    let path = auth_path(state);
    let current = read_managed_credential(&path)?;
    let active_id = current
        .as_ref()
        .map(credential_identity)
        .transpose()?
        .and_then(|identity| profile_id_for_identity(&connection, &identity).transpose())
        .transpose()?;

    if let (Some(profile_id), Some(credential)) = (active_id.as_deref(), current.as_ref()) {
        let auth_json =
            serde_json::to_string_pretty(credential).map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE accounts SET auth_json = ?1, updated_at = ?2 WHERE id = ?3 AND product = 'grok' AND auth_json <> ?1",
                params![auth_json, now_millis(), profile_id],
            )
            .map_err(database_error)?;
    }

    let profiles =
        list_profiles_for_product(&connection, AccountProduct::Grok, active_id.as_deref())?;
    let (kind, message) = if active_id.is_some() {
        ("managed", "当前 Grok 登录状态已匹配已保存账户。")
    } else if current.is_some() {
        ("unmanaged", "当前 Grok 登录状态尚未纳入本应用管理。")
    } else if has_nonstandard_credentials(&path)? {
        ("unmanaged", "检测到非标准 Grok 凭据，第一期暂不支持管理。")
    } else {
        ("missing", "尚未检测到 Grok OAuth 登录。")
    };
    Ok(AppStatus {
        profiles,
        detected_profile: current
            .as_ref()
            .filter(|_| active_id.is_none())
            .map(detected_profile)
            .transpose()?,
        auth_path: path.display().to_string(),
        auth_state: AuthState {
            kind: kind.to_string(),
            message: message.to_string(),
        },
        autostart_enabled: app.autolaunch().is_enabled().unwrap_or(false),
        web_access: local_web::web_access_status(app, state)?,
    })
}

pub(super) fn upsert_oauth_profile(
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
    let connection = open_database(state)?;
    let existing = profile_id_for_identity(&connection, &identity)?;
    let now = now_millis();
    let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
    let alias = alias_for(requested_alias, &identity);
    if connection
        .execute(
            "UPDATE accounts SET account_id = ?1, email = ?2, alias = CASE WHEN ?3 = '' THEN alias ELSE ?3 END, auth_json = ?4, updated_at = ?5 WHERE id = ?6 AND product = 'grok'",
            params![identity.account_id, identity.email, requested_alias.trim(), auth_json, now, id],
        )
        .map_err(database_error)?
        == 0
    {
        connection
            .execute(
                "INSERT INTO accounts (id, product, account_type, account_id, email, alias, plan_type, auth_json, created_at, updated_at, sort_order) VALUES (?1, 'grok', 'oauth', ?2, ?3, ?4, '', ?5, ?6, ?6, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'grok'), 0))",
                params![id, identity.account_id, identity.email, alias, auth_json, now],
            )
            .map_err(database_error)?;
    }
    let active_id = active_profile_id(&connection, &auth_path(state))?;
    get_profile_summary_for_product(&connection, AccountProduct::Grok, &id, active_id.as_deref())
}

pub(super) fn import_current_profile(
    state: &AppState,
    requested_alias: Option<String>,
) -> Result<ProfileSummary, String> {
    let credential = read_managed_credential(&auth_path(state))?
        .ok_or_else(|| "未找到受支持的 Grok xAI OAuth 凭据。".to_string())?;
    let identity = credential_identity(&credential)?;
    let auth_json = serde_json::to_string_pretty(&credential).map_err(|error| error.to_string())?;
    let connection = open_database(state)?;
    let existing = profile_id_for_identity(&connection, &identity)?;
    let now = now_millis();
    let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
    let requested_alias = requested_alias.unwrap_or_default();
    if connection
        .execute(
            "UPDATE accounts SET account_id = ?1, email = ?2, alias = CASE WHEN ?3 = '' THEN alias ELSE ?3 END, auth_json = ?4, updated_at = ?5 WHERE id = ?6 AND product = 'grok'",
            params![identity.account_id, identity.email, requested_alias.trim(), auth_json, now, id],
        )
        .map_err(database_error)?
        == 0
    {
        connection
            .execute(
                "INSERT INTO accounts (id, product, account_type, account_id, email, alias, plan_type, auth_json, created_at, updated_at, sort_order) VALUES (?1, 'grok', 'oauth', ?2, ?3, ?4, '', ?5, ?6, ?6, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts WHERE product = 'grok'), 0))",
                params![id, identity.account_id, identity.email, alias_for(&requested_alias, &identity), auth_json, now],
            )
            .map_err(database_error)?;
    }
    get_profile_summary_for_product(&connection, AccountProduct::Grok, &id, Some(&id))
}

pub(super) fn switch_profile(
    state: &AppState,
    profile_id: &str,
    force: bool,
) -> Result<ProfileSummary, String> {
    let _guard = SWITCH_LOCK
        .try_lock()
        .map_err(|_| "已有 Grok 账号切换正在进行，请稍后重试。".to_string())?;
    let connection = open_database(state)?;
    let path = auth_path(state);
    if active_profile_id(&connection, &path)?.is_none()
        && read_managed_credential(&path)?.is_some()
        && !force
    {
        return Err(
            "检测到工具外的 Grok 登录变更。请先同步当前账号，或确认后强制切换。".to_string(),
        );
    }
    let mut credential: Value = connection
        .query_row(
            "SELECT auth_json FROM accounts WHERE id = ?1 AND product = 'grok'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?
        .parse()
        .map_err(|_| "存档的 Grok 凭据已损坏。".to_string())?;
    refresh_credential_if_needed(&mut credential)?;
    write_managed_credential(&path, &credential)?;
    let auth_json = serde_json::to_string_pretty(&credential).map_err(|error| error.to_string())?;
    let now = now_millis();
    connection
        .execute(
            "UPDATE accounts SET auth_json = ?1, last_used_at = ?2, updated_at = ?2 WHERE id = ?3 AND product = 'grok'",
            params![auth_json, now, profile_id],
        )
        .map_err(database_error)?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Grok,
        profile_id,
        Some(profile_id),
    )
}

pub(super) fn update_alias(
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
            "UPDATE accounts SET alias = ?1, updated_at = ?2 WHERE id = ?3 AND product = 'grok'",
            params![alias, now_millis(), profile_id],
        )
        .map_err(database_error)?
        == 0
    {
        return Err("账户不存在。".to_string());
    }
    let active_id = active_profile_id(&connection, &auth_path(state))?;
    get_profile_summary_for_product(
        &connection,
        AccountProduct::Grok,
        profile_id,
        active_id.as_deref(),
    )
}

pub(super) fn refresh_profile_usage(
    state: &AppState,
    profile_id: &str,
) -> Result<ProfileSummary, String> {
    let connection = open_database(state)?;
    let auth_json = connection
        .query_row(
            "SELECT auth_json FROM accounts WHERE id = ?1 AND product = 'grok'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    let mut credential: Value =
        serde_json::from_str(&auth_json).map_err(|_| "存档的 Grok 凭据已损坏。".to_string())?;
    let previous_credential = credential.clone();
    refresh_credential_if_needed(&mut credential)?;
    let active_id = active_profile_id(&connection, &auth_path(state))?;
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
    let used_percent = config.credit_usage_percent.or_else(|| {
        let limit = config.monthly_limit.as_ref()?.val;
        (limit > 0).then(|| {
            config.used.as_ref().map(|value| value.val).unwrap_or(0) as f64 / limit as f64 * 100.0
        })
    })?;
    if !used_percent.is_finite() {
        return None;
    }
    let start = config
        .current_period
        .as_ref()
        .and_then(|period| period.start.as_deref())
        .or(config.billing_period_start.as_deref())
        .and_then(parse_billing_time);
    let end = config
        .current_period
        .as_ref()
        .and_then(|period| period.end.as_deref())
        .or(config.billing_period_end.as_deref())
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
    grok_home(state, std::env::var_os("GROK_HOME")).join("auth.json")
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
    grok_home(state, std::env::var_os("GROK_HOME")).join("config.toml")
}

#[tauri::command]
pub(super) fn get_grok_config(state: State<'_, AppState>) -> Result<ConfigFile, String> {
    read_grok_config(&grok_config_path(&state))
}

fn read_grok_config(path: &Path) -> Result<ConfigFile, String> {
    config::read_config(path, "", "Grok config.toml")
}

#[tauri::command]
pub(super) fn validate_grok_config(content: String) -> Vec<ConfigDiagnostic> {
    config::validate_toml(&content)
}

#[tauri::command]
pub(super) fn format_grok_config(content: String) -> Result<String, String> {
    config::format_toml(&content, "config.toml")
}

#[tauri::command]
pub(super) fn save_grok_config(state: State<'_, AppState>, content: String) -> Result<(), String> {
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

fn read_managed_credential(path: &Path) -> Result<Option<Value>, String> {
    Ok(read_store(path)?
        .get(grok_oauth::AUTH_REGISTRY_KEY)
        .cloned())
}

fn has_nonstandard_credentials(path: &Path) -> Result<bool, String> {
    Ok(read_store(path)?
        .keys()
        .any(|key| key != grok_oauth::AUTH_REGISTRY_KEY))
}

fn write_managed_credential(path: &Path, credential: &Value) -> Result<(), String> {
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
    let mut store = read_store(path)?;
    store.insert(
        grok_oauth::AUTH_REGISTRY_KEY.to_string(),
        credential.clone(),
    );
    let content = serde_json::to_string_pretty(&store).map_err(|error| error.to_string())?;
    write_file_atomically(path, &content)
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
            "SELECT id FROM accounts WHERE product = 'grok' AND (account_id = ?1 OR (?2 <> '' AND email = ?2)) LIMIT 1",
            params![identity.account_id, identity.email],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)
}

fn active_profile_id(connection: &Connection, path: &Path) -> Result<Option<String>, String> {
    let Some(credential) = read_managed_credential(path)? else {
        return Ok(None);
    };
    profile_id_for_identity(connection, &credential_identity(&credential)?)
}

fn detected_profile(credential: &Value) -> Result<ProfileSummary, String> {
    let identity = credential_identity(credential)?;
    let alias = alias_for("", &identity);
    let now = now_millis();
    Ok(ProfileSummary {
        id: "detected".to_string(),
        product: AccountProduct::Grok,
        account_type: ACCOUNT_TYPE_OAUTH.to_string(),
        api_base_url: None,
        account_id: identity.account_id,
        email: identity.email,
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
    use super::*;

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
    fn keeps_other_scopes_when_switching_grok_credential() {
        let directory = std::env::temp_dir().join(format!("cortana-grok-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("auth.json");
        write_file_atomically(&path, r#"{"xai::api_key":{"key":"keep"}}"#).unwrap();
        let credential = json!({"key":"token","user_id":"u1","email":"u@example.com"});
        write_managed_credential(&path, &credential).unwrap();
        let store = read_store(&path).unwrap();
        assert_eq!(store["xai::api_key"]["key"], "keep");
        assert_eq!(store[grok_oauth::AUTH_REGISTRY_KEY]["key"], "token");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn identity_and_alias_follow_grok_fallback_order() {
        let credential = json!({
            "principal_id": "team-1",
            "user_id": "user-1",
            "email": "person@example.com",
            "team_name": "团队",
            "first_name": "Lin"
        });
        let identity = credential_identity(&credential).unwrap();
        assert_eq!(identity.account_id, "team-1");
        assert_eq!(alias_for("", &identity), "团队");
        assert_eq!(alias_for("工作", &identity), "工作");
    }

    #[test]
    fn refuses_to_replace_an_unmanaged_grok_login() {
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
        assert!(switch_profile(&state, &target.id, false)
            .unwrap_err()
            .contains("工具外"));
        switch_profile(&state, &target.id, true).unwrap();
        assert_eq!(
            read_managed_credential(&path).unwrap().unwrap()["user_id"],
            "target-user"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn parses_grok_weekly_billing_usage_and_legacy_fallback() {
        let usage = parse_billing_usage(
            r#"{"config":{"creditUsagePercent":25.5,"currentPeriod":{"start":"2026-07-22T00:00:00Z","end":"2026-07-29T00:00:00Z"}},"subscriptionTier":"SuperGrok"}"#,
        )
        .unwrap();
        assert_eq!(usage.plan_type, "SuperGrok");
        let window = usage.primary.unwrap();
        assert_eq!(window.used_percent, 25.5);
        assert_eq!(window.window_minutes, Some(7 * 24 * 60));
        assert_eq!(window.resets_at, Some(1785283200000));

        let legacy = parse_billing_usage(
            r#"{"config":{"monthlyLimit":{"val":1000},"used":{"val":400},"billingPeriodEnd":"2026-08-01T00:00:00Z"}}"#,
        )
        .unwrap();
        assert_eq!(legacy.primary.unwrap().used_percent, 40.0);
    }

    #[test]
    fn accepts_billing_response_without_public_usage_percentage() {
        let usage = parse_billing_usage(
            r#"{"config":{"currentPeriod":{"start":"2026-07-22T00:00:00Z","end":"2026-07-29T00:00:00Z"}}}"#,
        )
        .unwrap();
        assert!(usage.primary.is_none());
    }
}
