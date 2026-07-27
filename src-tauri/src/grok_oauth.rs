use super::{grok, oauth, tray::*, *};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) const OIDC_ISSUER: &str = "https://auth.x.ai";
pub(super) const OIDC_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub(super) const OIDC_SCOPE: &str =
    "openid profile email offline_access grok-cli:access api:access conversations:read conversations:write workspaces:read workspaces:write";
pub(super) const AUTH_REGISTRY_KEY: &str =
    "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828";
pub(super) const DEFAULT_TOKEN_ENDPOINT: &str = "https://auth.x.ai/oauth2/token";
const DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const DEFAULT_POLL_INTERVAL: u64 = 5;
const MAX_LOGIN_SECONDS: i64 = 30 * 60;

static CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Deserialize)]
struct DiscoveryResponse {
    device_authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    userinfo_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: i64,
    #[serde(default)]
    interval: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GrokTokenResponse {
    pub(super) access_token: String,
    #[serde(default)]
    pub(super) refresh_token: Option<String>,
    #[serde(default)]
    pub(super) id_token: Option<String>,
    #[serde(default)]
    pub(super) expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UserInfoResponse {
    #[serde(default, alias = "userId")]
    pub(super) sub: Option<String>,
    #[serde(default)]
    pub(super) email: Option<String>,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default, alias = "firstName")]
    pub(super) given_name: Option<String>,
    #[serde(default, alias = "lastName")]
    pub(super) family_name: Option<String>,
    #[serde(default, alias = "principalType")]
    pub(super) principal_type: Option<String>,
    #[serde(default, alias = "principalId")]
    pub(super) principal_id: Option<String>,
    #[serde(default, alias = "teamId")]
    pub(super) team_id: Option<String>,
    #[serde(default, alias = "teamName")]
    pub(super) team_name: Option<String>,
}

pub(super) fn start_device_oauth_add(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    alias: Option<String>,
    activate: bool,
) -> Result<(), String> {
    CANCELLED.store(false, Ordering::SeqCst);

    let mut pending = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    if pending.is_some() {
        return Err("已有一个授权流程正在进行，请先完成或取消。".to_string());
    }
    *pending = Some(PendingOAuth {
        product: AccountProduct::Grok,
        alias: alias.unwrap_or_default().trim().to_string(),
        activate,
        code_verifier: String::new(),
        state: String::new(),
        callback_url: String::new(),
        exchanging: false,
    });
    drop(pending);

    oauth::emit_progress(
        &app,
        "browser_opening",
        "正在准备 Grok Device Code 授权。",
        None,
    );

    let app_for_thread = app.clone();
    let state_for_thread = state.inner().clone();
    thread::spawn(move || {
        if let Err(message) = run_device_login(&app_for_thread, &state_for_thread) {
            if CANCELLED.load(Ordering::SeqCst) {
                return;
            }
            oauth::clear_pending_oauth(&state_for_thread);
            oauth::emit_progress(&app_for_thread, "error", &message, None);
        }
    });
    Ok(())
}

pub(super) fn cancel_device_oauth() {
    CANCELLED.store(true, Ordering::SeqCst);
}

fn run_device_login(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let client = http_client()?;
    let discovery = discover(&client)?;
    let device = request_device_code(&client, &discovery.device_authorization_endpoint)?;
    let open_url = device
        .verification_uri_complete
        .clone()
        .unwrap_or_else(|| device.verification_uri.clone());

    oauth::emit_progress(
        app,
        "waiting",
        &format!("请在浏览器中完成授权。验证码：{}。", device.user_code),
        None,
    );
    let _ = app.opener().open_url(open_url, None::<&str>);

    let expires_at =
        Instant::now() + Duration::from_secs(device.expires_in.clamp(1, MAX_LOGIN_SECONDS) as u64);
    let mut interval = Duration::from_secs(
        device
            .interval
            .unwrap_or(DEFAULT_POLL_INTERVAL)
            .max(DEFAULT_POLL_INTERVAL),
    );

    while Instant::now() < expires_at {
        if CANCELLED.load(Ordering::SeqCst)
            || state
                .pending_oauth
                .lock()
                .map(|pending| pending.is_none())
                .unwrap_or(true)
        {
            return Ok(());
        }
        match poll_token(&client, &discovery.token_endpoint, &device.device_code)? {
            PollResult::Pending => thread::sleep(interval),
            PollResult::SlowDown => {
                interval += Duration::from_secs(5);
                thread::sleep(interval);
            }
            PollResult::Complete(token) => {
                oauth::emit_progress(app, "exchanging", "正在保存 Grok 账户。", None);
                let userinfo = fetch_userinfo(
                    &client,
                    discovery.userinfo_endpoint.as_deref(),
                    &token.access_token,
                )
                .ok();
                let (alias, activate) = {
                    let pending = state
                        .pending_oauth
                        .lock()
                        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
                    let pending = pending
                        .as_ref()
                        .ok_or_else(|| "OAuth 授权已取消。".to_string())?;
                    (pending.alias.clone(), pending.activate)
                };
                let profile = grok::upsert_oauth_profile(state, &token, userinfo.as_ref(), &alias)?;
                let profile = if activate {
                    grok::switch_profile(state, &profile.id, true)?
                } else {
                    profile
                };
                oauth::clear_pending_oauth(state);
                CANCELLED.store(false, Ordering::SeqCst);
                refresh_tray(app)?;
                oauth::emit_progress(app, "success", "Grok 账户已添加。", Some(profile));
                return Ok(());
            }
        }
    }
    Err("Grok 授权等待已超时，请重新开始。".to_string())
}

fn http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Cortana/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())
}

fn discover(client: &Client) -> Result<DiscoveryResponse, String> {
    let response = client
        .get(DISCOVERY_URL)
        .header("Accept", "application/json")
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("无法加载 Grok OAuth discovery：{error}"))?;
    response
        .json()
        .map_err(|error| format!("解析 Grok OAuth discovery 失败：{error}"))
}

fn request_device_code(client: &Client, endpoint: &str) -> Result<DeviceCodeResponse, String> {
    let response = client
        .post(endpoint)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("x-grok-client-version", env!("CARGO_PKG_VERSION"))
        .header("x-grok-client-surface", "ui")
        .form(&[
            ("client_id", OIDC_CLIENT_ID),
            ("scope", OIDC_SCOPE),
            ("referrer", "grok-build"),
        ])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("发起 Grok Device Code 失败：{error}"))?;
    let device: DeviceCodeResponse = response
        .json()
        .map_err(|error| format!("解析 Grok Device Code 响应失败：{error}"))?;
    if device.device_code.trim().is_empty()
        || device.user_code.trim().is_empty()
        || device.verification_uri.trim().is_empty()
    {
        return Err("Grok Device Code 响应缺少必要字段。".to_string());
    }
    if !device
        .user_code
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("Grok Device Code 返回了无效验证码。".to_string());
    }
    for uri in std::iter::once(device.verification_uri.as_str())
        .chain(device.verification_uri_complete.as_deref())
    {
        let url = Url::parse(uri).map_err(|_| "Grok 授权地址无效。".to_string())?;
        if url.scheme() != "https" || url.host_str().is_none() {
            return Err("Grok 授权地址必须使用 HTTPS。".to_string());
        }
    }
    Ok(device)
}

enum PollResult {
    Pending,
    SlowDown,
    Complete(GrokTokenResponse),
}

fn poll_token(
    client: &Client,
    token_endpoint: &str,
    device_code: &str,
) -> Result<PollResult, String> {
    let response = client
        .post(token_endpoint)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("x-grok-client-version", env!("CARGO_PKG_VERSION"))
        .header("x-grok-client-surface", "ui")
        .form(&[
            ("grant_type", DEVICE_GRANT_TYPE),
            ("device_code", device_code),
            ("client_id", OIDC_CLIENT_ID),
        ])
        .send()
        .map_err(|error| format!("轮询 Grok token 失败：{error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("读取 Grok token 响应失败：{error}"))?;
    if status.is_success() {
        let token: GrokTokenResponse = serde_json::from_str(&body)
            .map_err(|error| format!("解析 Grok token 失败：{error}"))?;
        if token.access_token.trim().is_empty() {
            return Err("Grok OAuth 未返回 access_token。".to_string());
        }
        return Ok(PollResult::Complete(token));
    }
    let error: TokenErrorResponse = serde_json::from_str(&body).unwrap_or(TokenErrorResponse {
        error: None,
        error_description: None,
    });
    match error.error.as_deref() {
        Some("authorization_pending") => Ok(PollResult::Pending),
        Some("slow_down") => Ok(PollResult::SlowDown),
        Some("access_denied") => Err("Grok OAuth 授权已被拒绝。".to_string()),
        Some("expired_token") => Err("Grok 验证码已过期，请重新开始。".to_string()),
        Some(code) => Err(format!(
            "Grok OAuth 失败：{}{}",
            code,
            error
                .error_description
                .as_deref()
                .map(|value| format!("（{value}）"))
                .unwrap_or_default()
        )),
        None => Err(format!("Grok OAuth 失败（HTTP {}）。", status.as_u16())),
    }
}

fn fetch_userinfo(
    client: &Client,
    endpoint: Option<&str>,
    access_token: &str,
) -> Result<UserInfoResponse, String> {
    let proxy_response = client
        .get("https://cli-chat-proxy.grok.com/v1/user")
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("X-XAI-Token-Auth", "xai-grok-cli")
        .header("x-grok-client-version", env!("CARGO_PKG_VERSION"))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status);
    if let Ok(response) = proxy_response {
        return response
            .json()
            .map_err(|error| format!("解析 Grok 用户信息失败：{error}"));
    }
    client
        .get(endpoint.unwrap_or("https://auth.x.ai/oauth2/userinfo"))
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("读取 Grok userinfo 失败：{error}"))?
        .json()
        .map_err(|error| format!("解析 Grok userinfo 失败：{error}"))
}

pub(super) fn refresh_access_token(refresh_token: &str) -> Result<GrokTokenResponse, String> {
    let client = http_client()?;
    let response = client
        .post(DEFAULT_TOKEN_ENDPOINT)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", OIDC_CLIENT_ID),
        ])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("刷新 Grok token 失败：{error}"))?;
    let token: GrokTokenResponse = response
        .json()
        .map_err(|error| format!("解析 Grok refresh 响应失败：{error}"))?;
    if token.access_token.trim().is_empty() {
        return Err("Grok refresh 未返回 access_token。".to_string());
    }
    Ok(token)
}

pub(super) type GrokUserInfo = UserInfoResponse;
