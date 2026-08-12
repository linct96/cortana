use super::store::{switch_profile_internal, upsert_profile_from_auth};
use crate::{
    platform::{
        local_web,
        state::{
            AccountProduct, AppState, Identity, OAuthProgress, OAuthTokenResponse, PendingOAuth,
            ProfileSummary, ANTIGRAVITY_OAUTH_AUTHORIZE_URL, ANTIGRAVITY_OAUTH_CLIENT_ID,
            ANTIGRAVITY_OAUTH_CLIENT_SECRET, ANTIGRAVITY_OAUTH_SCOPE, ANTIGRAVITY_OAUTH_TOKEN_URL,
            OAUTH_AUTHORIZE_URL, OAUTH_CALLBACK_URL, OAUTH_CLIENT_ID, OAUTH_SCOPE, OAUTH_TIMEOUT,
            OAUTH_TOKEN_URL,
        },
        tray::refresh_tray,
    },
    products::{antigravity, claude, codex::extract_refresh_token, grok::oauth as grok_oauth},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{SecondsFormat, Utc};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};
use tauri::{Emitter, State};
use tauri_plugin_opener::OpenerExt;
use url::Url;

enum OAuthExchange {
    Standard(OAuthTokenResponse),
    Claude(claude::ClaudeOAuthTokenResponse),
}

pub(crate) async fn import_auth_json(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    auth_json: String,
    alias: Option<String>,
    activate: bool,
) -> Result<ProfileSummary, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        import_auth_json_internal(&app, &state, &auth_json, alias, activate)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn import_auth_json_internal(
    app: &tauri::AppHandle,
    state: &AppState,
    auth_json: &str,
    alias: Option<String>,
    activate: bool,
) -> Result<ProfileSummary, String> {
    let refresh_token = extract_refresh_token(auth_json)?;
    let token = refresh_oauth_token(&refresh_token)?;
    let refreshed_auth_json = build_codex_auth_json(&token)?;
    let identity = serde_json::from_str(&refreshed_auth_json)
        .map(|auth| identity_from_auth_json(&auth))
        .map_err(|_| "刷新后的认证信息无效。".to_string())?;
    if identity.account_id.is_empty() {
        return Err("刷新后的认证信息缺少账户标识。".to_string());
    }
    let profile = upsert_profile_from_auth(
        state,
        &refreshed_auth_json,
        alias.unwrap_or_default().trim(),
    )?;
    let profile = if activate {
        switch_profile_internal(state, &profile.id, true)?
    } else {
        profile
    };
    refresh_tray(app)?;
    Ok(profile)
}

pub(crate) async fn start_oauth_add(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    alias: Option<String>,
    activate: bool,
    product: AccountProduct,
) -> Result<String, String> {
    if product == AccountProduct::Grok {
        let state = state.inner().clone();
        return tauri::async_runtime::spawn_blocking(move || {
            grok_oauth::start_device_oauth_add(app, state, alias, activate)
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    let mut pending = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    if let Some(current) = pending.as_mut() {
        if current.product != product {
            return Err("已有另一个产品的授权流程正在进行，请先取消。".to_string());
        }
        if current.exchanging {
            return Err("正在处理 OAuth 回调，请稍候。".to_string());
        }
        current.alias = alias.unwrap_or_default().trim().to_string();
        current.activate = activate;
        current.code_verifier = random_urlsafe(32);
        current.state = random_urlsafe(32);
        let auth_url = build_authorize_url(
            product,
            &current.code_verifier,
            &current.state,
            &current.callback_url,
        )?;
        emit_progress(&app, "waiting", "新的授权链接已生成，旧链接已失效。", None);
        return Ok(auth_url);
    }

    let (listeners, callback_url) = bind_oauth_listeners(product)?;
    let code_verifier = random_urlsafe(32);
    let state_token = random_urlsafe(32);
    let auth_url = build_authorize_url(product, &code_verifier, &state_token, &callback_url)?;
    *pending = Some(PendingOAuth {
        product,
        alias: alias.unwrap_or_default().trim().to_string(),
        activate,
        code_verifier,
        state: state_token,
        callback_url,
        exchanging: false,
    });
    drop(pending);

    emit_progress(
        &app,
        "waiting",
        &format!("{} 授权链接已生成。", product.display_name()),
        None,
    );

    let state_for_thread = state.inner().clone();
    thread::spawn(move || wait_for_oauth_callback(app, state_for_thread, listeners, product));
    Ok(auth_url)
}

pub(crate) fn open_oauth_add(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    authorization_url: String,
) -> Result<(), String> {
    let pending = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    let current = pending
        .as_ref()
        .ok_or_else(|| "OAuth 授权状态不存在或已过期。".to_string())?;
    let expected = pending_authorization_url(current)?;
    if authorization_url != expected {
        return Err("授权链接已失效，请重新生成。".to_string());
    }
    app.opener()
        .open_url(authorization_url, None::<&str>)
        .map_err(|error| format!("无法打开默认浏览器：{error}"))
}

fn pending_authorization_url(pending: &PendingOAuth) -> Result<String, String> {
    if pending.product == AccountProduct::Grok {
        Ok(pending.callback_url.clone())
    } else {
        build_authorize_url(
            pending.product,
            &pending.code_verifier,
            &pending.state,
            &pending.callback_url,
        )
    }
}

pub(crate) fn update_oauth_alias(state: State<'_, AppState>, alias: String) -> Result<(), String> {
    let mut pending = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    let current = pending
        .as_mut()
        .ok_or_else(|| "OAuth 授权状态不存在或已过期。".to_string())?;
    current.alias = alias.trim().to_string();
    Ok(())
}

pub(crate) async fn complete_oauth_add(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    callback_url: String,
) -> Result<ProfileSummary, String> {
    let state = state.inner().clone();
    let app_for_task = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        complete_oauth_callback(&app_for_task, &state, callback_url)
    })
    .await
    .map_err(|error| error.to_string())?;
    match &result {
        Ok(profile) => emit_progress(&app, "success", "账户已添加。", Some(profile.clone())),
        Err(message) => emit_progress(&app, "error", message, None),
    }
    result
}

pub(crate) fn cancel_oauth_add(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    grok_oauth::cancel_device_oauth();
    clear_pending_oauth(&state);
    emit_progress(&app, "cancelled", "授权已取消。", None);
    Ok(())
}

pub(crate) fn build_authorize_url(
    product: AccountProduct,
    code_verifier: &str,
    state: &str,
    callback_url: &str,
) -> Result<String, String> {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
    let authorize_url = match product {
        AccountProduct::Codex => OAUTH_AUTHORIZE_URL,
        AccountProduct::Claude => claude::CLAUDE_OAUTH_AUTHORIZE_URL,
        AccountProduct::Antigravity => ANTIGRAVITY_OAUTH_AUTHORIZE_URL,
        AccountProduct::Grok => return Err("Grok 使用 Device Code 授权。".to_string()),
    };
    let mut url = Url::parse(authorize_url).map_err(|error| error.to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair(
            "client_id",
            match product {
                AccountProduct::Codex => OAUTH_CLIENT_ID,
                AccountProduct::Claude => claude::CLAUDE_OAUTH_CLIENT_ID,
                AccountProduct::Antigravity => ANTIGRAVITY_OAUTH_CLIENT_ID,
                AccountProduct::Grok => unreachable!(),
            },
        );
        query.append_pair("redirect_uri", callback_url);
        query.append_pair(
            "scope",
            match product {
                AccountProduct::Codex => OAUTH_SCOPE,
                AccountProduct::Claude => claude::CLAUDE_OAUTH_SCOPE,
                AccountProduct::Antigravity => ANTIGRAVITY_OAUTH_SCOPE,
                AccountProduct::Grok => unreachable!(),
            },
        );
        query.append_pair("code_challenge", &challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("state", state);
        match product {
            AccountProduct::Codex => {
                query.append_pair("id_token_add_organizations", "true");
                query.append_pair("codex_cli_simplified_flow", "true");
                query.append_pair("originator", "codex_cli_rs");
            }
            AccountProduct::Claude => {
                query.append_pair("code", "true");
            }
            AccountProduct::Antigravity => {
                query.append_pair("access_type", "offline");
                query.append_pair("prompt", "consent");
                query.append_pair("include_granted_scopes", "true");
            }
            AccountProduct::Grok => unreachable!(),
        }
    }
    Ok(url.into())
}

pub(crate) fn bind_oauth_listeners(
    product: AccountProduct,
) -> Result<(Vec<TcpListener>, String), String> {
    if product == AccountProduct::Antigravity {
        let mut listeners = Vec::new();
        let (port, first_is_ipv6) = match TcpListener::bind("[::1]:0") {
            Ok(listener) => {
                let port = listener
                    .local_addr()
                    .map_err(|error| error.to_string())?
                    .port();
                listeners.push(listener);
                (port, true)
            }
            Err(_) => {
                let listener = TcpListener::bind("127.0.0.1:0")
                    .map_err(|error| format!("无法监听本地 OAuth 回调：{error}"))?;
                let port = listener
                    .local_addr()
                    .map_err(|error| error.to_string())?
                    .port();
                listeners.push(listener);
                (port, false)
            }
        };
        let second_address = if first_is_ipv6 {
            format!("127.0.0.1:{port}")
        } else {
            format!("[::1]:{port}")
        };
        if let Ok(listener) = TcpListener::bind(second_address) {
            listeners.push(listener);
        }
        for listener in &listeners {
            listener
                .set_nonblocking(true)
                .map_err(|error| error.to_string())?;
        }
        let host = if listeners.len() == 2 {
            "localhost"
        } else if first_is_ipv6 {
            "[::1]"
        } else {
            "127.0.0.1"
        };
        return Ok((listeners, format!("http://{host}:{port}/auth/callback")));
    }
    if product == AccountProduct::Claude {
        let mut listeners = Vec::new();
        for address in ["127.0.0.1:54545", "[::1]:54545"] {
            if let Ok(listener) = TcpListener::bind(address) {
                listener
                    .set_nonblocking(true)
                    .map_err(|error| error.to_string())?;
                listeners.push(listener);
            }
        }
        if listeners.is_empty() {
            return Err("无法监听 localhost:54545。请关闭占用该端口的程序后重试。".to_string());
        }
        return Ok((listeners, claude::CLAUDE_OAUTH_CALLBACK_URL.to_string()));
    }
    let mut listeners = Vec::new();
    for address in ["127.0.0.1:1455", "[::1]:1455"] {
        if let Ok(listener) = TcpListener::bind(address) {
            listener
                .set_nonblocking(true)
                .map_err(|error| error.to_string())?;
            listeners.push(listener);
        }
    }
    if listeners.is_empty() {
        return Err("无法监听 localhost:1455。请关闭占用该端口的程序后重试。".to_string());
    }
    Ok((listeners, OAUTH_CALLBACK_URL.to_string()))
}

pub(crate) fn wait_for_oauth_callback(
    app: tauri::AppHandle,
    state: AppState,
    listeners: Vec<TcpListener>,
    product: AccountProduct,
) {
    emit_progress(
        &app,
        "waiting",
        "请在浏览器中完成授权，应用会自动接收结果。",
        None,
    );
    let started_at = Instant::now();
    while started_at.elapsed() < OAUTH_TIMEOUT {
        if state
            .pending_oauth
            .lock()
            .map(|pending| pending.is_none())
            .unwrap_or(true)
        {
            return;
        }
        for listener in &listeners {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let callback_url = state.pending_oauth.lock().ok().and_then(|pending| {
                        pending.as_ref().map(|value| value.callback_url.clone())
                    });
                    let callback = callback_url
                        .as_ref()
                        .ok_or_else(|| "OAuth 授权已取消。".to_string())
                        .and_then(|callback_url| read_callback_url(&mut stream, callback_url));
                    match callback.and_then(|url| complete_oauth_callback(&app, &state, url)) {
                        Ok(profile) => {
                            let _ = write_browser_response(
                                &mut stream,
                                product,
                                true,
                                "授权完成，可以返回 Cortana。 ",
                            );
                            emit_progress(&app, "success", "账户已添加。", Some(profile));
                        }
                        Err(message) => {
                            let _ = write_browser_response(
                                &mut stream,
                                product,
                                false,
                                "授权未完成，可以回到应用重试。 ",
                            );
                            let pending = state
                                .pending_oauth
                                .lock()
                                .map(|pending| pending.is_some())
                                .unwrap_or(false);
                            if pending {
                                continue;
                            }
                            if message != "OAuth 授权已取消。" {
                                emit_progress(&app, "error", &message, None);
                            }
                        }
                    }
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => {
                    clear_pending_oauth(&state);
                    emit_progress(
                        &app,
                        "error",
                        &format!("本地 OAuth 回调服务失败：{error}"),
                        None,
                    );
                    return;
                }
            }
        }
        thread::sleep(Duration::from_millis(150));
    }
    clear_pending_oauth(&state);
    emit_progress(&app, "error", "授权等待已超时，请重新开始。", None);
}

pub(crate) fn read_callback_url(
    stream: &mut TcpStream,
    callback_url: &str,
) -> Result<String, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    let mut buffer = [0_u8; 8192];
    let length = stream
        .read(&mut buffer)
        .map_err(|_| "未能读取 OAuth 回调。".to_string())?;
    let request = String::from_utf8_lossy(&buffer[..length]);
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| "OAuth 回调请求无效。".to_string())?;
    let callback = Url::parse(callback_url).map_err(|_| "OAuth 回调地址无效。".to_string())?;
    let origin = format!(
        "{}://{}:{}",
        callback.scheme(),
        callback.host_str().unwrap_or("localhost"),
        callback.port_or_known_default().unwrap_or(80)
    );
    Ok(format!("{origin}{target}"))
}

pub(crate) fn complete_oauth_callback(
    app: &tauri::AppHandle,
    state: &AppState,
    callback_url: String,
) -> Result<ProfileSummary, String> {
    let pending_guard = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    let pending_callback = pending_guard
        .as_ref()
        .ok_or_else(|| "OAuth 授权状态不存在或已过期。".to_string())?;
    if pending_callback.exchanging {
        return Err("正在处理 OAuth 回调，请稍候。".to_string());
    }
    let callback = validate_callback_url(&callback_url, pending_callback)?;
    let error = callback
        .query_pairs()
        .find(|(key, _)| key == "error")
        .map(|(_, value)| value.into_owned());
    if let Some(error) = error {
        drop(pending_guard);
        clear_pending_oauth(state);
        return Err(format!("OAuth 授权被拒绝：{error}"));
    }
    let code = callback
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| "OAuth 回调缺少授权 code。".to_string())?;
    let pending = pending_guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "OAuth 授权状态不存在或已过期。".to_string())?;
    drop(pending_guard);
    let mut pending_guard = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    let current = pending_guard
        .as_mut()
        .filter(|current| current.state == pending.state)
        .ok_or_else(|| "OAuth 授权状态不存在或已过期。".to_string())?;
    if current.exchanging {
        return Err("正在处理 OAuth 回调，请稍候。".to_string());
    }
    current.exchanging = true;
    drop(pending_guard);
    emit_progress(app, "exchanging", "正在交换授权信息。", None);
    let exchange = match pending.product {
        AccountProduct::Claude => OAuthExchange::Claude(
            claude::exchange_code(
                &code,
                &pending.state,
                &pending.code_verifier,
                &pending.callback_url,
            )
            .inspect_err(|_| clear_pending_oauth(state))?,
        ),
        AccountProduct::Codex | AccountProduct::Antigravity => OAuthExchange::Standard(
            exchange_code(
                pending.product,
                &code,
                &pending.code_verifier,
                &pending.callback_url,
            )
            .inspect_err(|_| clear_pending_oauth(state))?,
        ),
        AccountProduct::Grok => return Err("Grok 使用 Device Code 授权。".to_string()),
    };
    let mut pending_guard = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    if pending_guard.as_ref().map(|current| current.state.as_str()) != Some(&pending.state) {
        return Err("OAuth 授权已取消。".to_string());
    }
    pending_guard.take();
    drop(pending_guard);
    let profile = match (pending.product, exchange) {
        (AccountProduct::Codex, OAuthExchange::Standard(token)) => {
            let auth_json = build_codex_auth_json(&token)?;
            let profile = upsert_profile_from_auth(state, &auth_json, &pending.alias)?;
            if pending.activate {
                switch_profile_internal(state, &profile.id, true)?
            } else {
                profile
            }
        }
        (AccountProduct::Claude, OAuthExchange::Claude(token)) => {
            let profile = claude::upsert_oauth_profile(state, token, &pending.alias)?;
            if pending.activate {
                claude::switch_profile(state, &profile.id, true)?
            } else {
                profile
            }
        }
        (AccountProduct::Antigravity, OAuthExchange::Standard(token)) => {
            antigravity::upsert_oauth_profile(state, &token, &pending.alias)?
        }
        (AccountProduct::Grok, _) => return Err("Grok 使用 Device Code 授权。".to_string()),
        _ => return Err("OAuth 授权响应与产品不匹配。".to_string()),
    };
    refresh_tray(app)?;
    Ok(profile)
}

fn validate_callback_url(callback_url: &str, pending: &PendingOAuth) -> Result<Url, String> {
    let callback = Url::parse(callback_url).map_err(|_| "OAuth 回调地址无效。".to_string())?;
    let expected =
        Url::parse(&pending.callback_url).map_err(|_| "OAuth 回调地址无效。".to_string())?;
    if callback.scheme() != expected.scheme()
        || callback.host_str() != expected.host_str()
        || callback.port_or_known_default() != expected.port_or_known_default()
        || callback.path() != expected.path()
    {
        return Err("OAuth 回调地址不受信任。".to_string());
    }
    let received_state = callback
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| "OAuth 回调缺少 state。".to_string())?;
    if pending.state != received_state {
        return Err("OAuth state 不匹配，已拒绝本次回调。".to_string());
    }
    Ok(callback)
}

pub(crate) struct OAuthRefreshError {
    pub(crate) message: String,
    pub(crate) reauthorization_required: bool,
}

pub(crate) fn refresh_oauth_token(refresh_token: &str) -> Result<OAuthTokenResponse, String> {
    refresh_oauth_token_detailed(refresh_token).map_err(|error| error.message)
}

pub(crate) fn refresh_oauth_token_detailed(
    refresh_token: &str,
) -> Result<OAuthTokenResponse, OAuthRefreshError> {
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| OAuthRefreshError {
            message: error.to_string(),
            reauthorization_required: false,
        })?;
    let response = client
        .post(OAUTH_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", OAUTH_CLIENT_ID),
        ])
        .send()
        .map_err(|error| OAuthRefreshError {
            message: format!("认证信息刷新请求失败：{error}"),
            reauthorization_required: false,
        })?;
    let status = response.status();
    let body = response.text().map_err(|error| OAuthRefreshError {
        message: error.to_string(),
        reauthorization_required: false,
    })?;
    if !status.is_success() {
        return Err(OAuthRefreshError {
            message: format!("认证信息刷新失败（HTTP {}）。", status.as_u16()),
            reauthorization_required: oauth_refresh_requires_reauthorization(
                status.as_u16(),
                &body,
            ),
        });
    }
    let mut token: OAuthTokenResponse =
        serde_json::from_str(&body).map_err(|_| OAuthRefreshError {
            message: "认证信息刷新响应不是有效 JSON。".to_string(),
            reauthorization_required: false,
        })?;
    if token.access_token.as_deref().unwrap_or_default().is_empty() {
        return Err(OAuthRefreshError {
            message: "认证信息刷新未返回 access_token。".to_string(),
            reauthorization_required: false,
        });
    }
    if token
        .refresh_token
        .as_deref()
        .unwrap_or_default()
        .is_empty()
    {
        token.refresh_token = Some(refresh_token.to_string());
    }
    Ok(token)
}

fn oauth_refresh_requires_reauthorization(status: u16, body: &str) -> bool {
    if !matches!(status, 400 | 401) {
        return false;
    }
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value.get("error")?.as_str().map(str::to_string))
        .is_some_and(|error| matches!(error.as_str(), "invalid_grant" | "invalid_token"))
}

pub(crate) fn build_codex_auth_json(token: &OAuthTokenResponse) -> Result<String, String> {
    let access_token = token.access_token.as_deref().unwrap_or_default().trim();
    if access_token.is_empty() {
        return Err("OAuth token 缺少 access_token。".to_string());
    }
    let mut identity = token
        .id_token
        .as_deref()
        .map(identity_from_id_token)
        .unwrap_or_default();
    if identity.account_id.is_empty() {
        identity = identity_from_jwt(access_token);
    }
    serde_json::to_string_pretty(&json!({
        "access_token": access_token,
        "account_id": identity.account_id,
        "id_token": token.id_token.as_deref().unwrap_or_default(),
        "last_refresh": Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true),
        "refresh_token": token.refresh_token.as_deref().unwrap_or_default(),
        "type": "codex",
    }))
    .map_err(|error| error.to_string())
}

pub(crate) fn identity_from_auth_json(auth: &Value) -> Identity {
    let id_token = auth
        .get("id_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut identity = identity_from_id_token(id_token);
    if let Some(account_id) = auth
        .get("account_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|account_id| !account_id.is_empty())
    {
        identity.account_id = account_id.to_string();
    }
    let access_token = auth
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let access_identity = identity_from_jwt(access_token);
    if identity.account_id.is_empty() {
        identity.account_id = access_identity.account_id;
    }
    if identity.email.is_empty() {
        identity.email = access_identity.email;
    }
    if identity.plan_type.is_empty() {
        identity.plan_type = access_identity.plan_type;
    }
    identity
}

pub(crate) fn identity_from_id_token(token: &str) -> Identity {
    let mut identity = identity_from_jwt(token);
    identity.name = decode_jwt_claims(token)
        .and_then(|claims| {
            claims
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    identity
}

pub(crate) fn identity_from_jwt(token: &str) -> Identity {
    let claims = decode_jwt_claims(token);
    let account_id = claims
        .as_ref()
        .and_then(|claims| claims.get("https://api.openai.com/auth"))
        .and_then(Value::as_object)
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let email = claims
        .as_ref()
        .and_then(|claims| {
            claims.get("email").or_else(|| {
                claims
                    .get("https://api.openai.com/profile")
                    .and_then(|profile| profile.get("email"))
            })
        })
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let plan_type = claims
        .as_ref()
        .and_then(|claims| claims.get("https://api.openai.com/auth"))
        .and_then(Value::as_object)
        .and_then(|auth| auth.get("chatgpt_plan_type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Identity {
        account_id,
        name: String::new(),
        email,
        plan_type,
    }
}

pub(crate) fn chatgpt_user_id_from_auth_json(auth: &Value) -> String {
    ["id_token", "access_token"]
        .into_iter()
        .filter_map(|key| auth.get(key)?.as_str())
        .find_map(|token| {
            let claims = decode_jwt_claims(token)?;
            let auth = claims.get("https://api.openai.com/auth")?.as_object()?;
            auth.get("chatgpt_user_id")
                .or_else(|| auth.get("user_id"))?
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_default()
}

pub(crate) fn decode_jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&decoded).ok()
}

pub(crate) fn random_urlsafe(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    rand::fill(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

pub(crate) fn exchange_code(
    product: AccountProduct,
    code: &str,
    verifier: &str,
    callback_url: &str,
) -> Result<OAuthTokenResponse, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())?;
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", callback_url),
        (
            "client_id",
            match product {
                AccountProduct::Codex => OAUTH_CLIENT_ID,
                AccountProduct::Claude => {
                    return Err("Claude 使用 JSON OAuth token 请求。".to_string())
                }
                AccountProduct::Antigravity => ANTIGRAVITY_OAUTH_CLIENT_ID,
                AccountProduct::Grok => return Err("Grok 使用 Device Code 授权。".to_string()),
            },
        ),
        ("code_verifier", verifier),
    ];
    if product == AccountProduct::Antigravity {
        form.push(("client_secret", ANTIGRAVITY_OAUTH_CLIENT_SECRET));
    }
    let response = client
        .post(match product {
            AccountProduct::Codex => OAUTH_TOKEN_URL,
            AccountProduct::Claude => return Err("Claude 使用 JSON OAuth token 请求。".to_string()),
            AccountProduct::Antigravity => ANTIGRAVITY_OAUTH_TOKEN_URL,
            AccountProduct::Grok => return Err("Grok 使用 Device Code 授权。".to_string()),
        })
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .map_err(|error| format!("OAuth code exchange 请求失败：{error}"))?;
    let status = response.status();
    let body = response.text().map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "OAuth code exchange 失败（HTTP {}）。",
            status.as_u16()
        ));
    }
    let token: OAuthTokenResponse =
        serde_json::from_str(&body).map_err(|_| "OAuth token 响应不是有效 JSON。".to_string())?;
    if token.access_token.as_deref().unwrap_or_default().is_empty() {
        return Err("OAuth code exchange 未返回 access_token。".to_string());
    }
    Ok(token)
}

pub(crate) fn clear_pending_oauth(state: &AppState) {
    if let Ok(mut pending) = state.pending_oauth.lock() {
        *pending = None;
    }
}

pub(crate) fn emit_progress(
    app: &tauri::AppHandle,
    stage: &str,
    message: &str,
    profile: Option<ProfileSummary>,
) {
    let progress = OAuthProgress {
        stage: stage.to_string(),
        message: message.to_string(),
        profile,
    };
    local_web::record_oauth_progress(
        app,
        progress.clone(),
        matches!(stage, "browser_opening" | "waiting" | "exchanging"),
    );
    let _ = app.emit("oauth-progress", progress);
}

pub(crate) fn write_browser_response(
    stream: &mut TcpStream,
    product: AccountProduct,
    success: bool,
    message: &str,
) -> std::io::Result<()> {
    let product_name = product.display_name();
    let title = if success {
        format!("{product_name} 授权完成")
    } else {
        format!("{product_name} 授权未完成")
    };
    let color = if success { "#137a4b" } else { "#b73b2d" };
    let body = format!("<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><title>{title}</title><body style=\"font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;margin:48px;color:#1a232b\"><h1 style=\"color:{color}\">{title}</h1><p>{message}</p></body></html>");
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
}

#[cfg(test)]
mod tests {
    use super::{build_authorize_url, pending_authorization_url, validate_callback_url};
    use crate::{
        platform::state::{
            AccountProduct, PendingOAuth, ANTIGRAVITY_OAUTH_AUTHORIZE_URL, OAUTH_AUTHORIZE_URL,
            OAUTH_CALLBACK_URL, OAUTH_CLIENT_ID,
        },
        products::claude,
    };
    use url::Url;

    #[test]
    fn oauth_authorize_url_matches_the_codex_cli_contract() {
        let url = Url::parse(
            &build_authorize_url(
                AccountProduct::Codex,
                "verifier",
                "state-value",
                OAUTH_CALLBACK_URL,
            )
            .unwrap(),
        )
        .unwrap();
        let query = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(url.as_str().split('?').next(), Some(OAUTH_AUTHORIZE_URL));
        assert_eq!(query.get("client_id"), Some(&OAUTH_CLIENT_ID.into()));
        assert_eq!(query.get("redirect_uri"), Some(&OAUTH_CALLBACK_URL.into()));
        assert_eq!(query.get("state"), Some(&"state-value".into()));
        assert_eq!(query.get("code_challenge_method"), Some(&"S256".into()));
        assert_eq!(query.get("codex_cli_simplified_flow"), Some(&"true".into()));
        assert_eq!(query.get("originator"), Some(&"codex_cli_rs".into()));
    }

    #[test]
    fn antigravity_authorize_url_uses_google_offline_pkce_flow() {
        let callback = "http://localhost:23456/auth/callback";
        let url = Url::parse(
            &build_authorize_url(
                AccountProduct::Antigravity,
                "verifier",
                "state-value",
                callback,
            )
            .unwrap(),
        )
        .unwrap();
        let query = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(
            url.as_str().split('?').next(),
            Some(ANTIGRAVITY_OAUTH_AUTHORIZE_URL)
        );
        assert_eq!(query.get("redirect_uri"), Some(&callback.into()));
        assert_eq!(query.get("state"), Some(&"state-value".into()));
        assert_eq!(query.get("code_challenge_method"), Some(&"S256".into()));
        assert_eq!(query.get("access_type"), Some(&"offline".into()));
        assert_eq!(query.get("prompt"), Some(&"consent".into()));
        assert_eq!(query.get("include_granted_scopes"), Some(&"true".into()));
    }

    #[test]
    fn claude_authorize_url_matches_the_claude_code_contract() {
        let url = Url::parse(
            &build_authorize_url(
                AccountProduct::Claude,
                "verifier",
                "state-value",
                claude::CLAUDE_OAUTH_CALLBACK_URL,
            )
            .unwrap(),
        )
        .unwrap();
        let query = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(
            url.as_str().split('?').next(),
            Some(claude::CLAUDE_OAUTH_AUTHORIZE_URL)
        );
        assert_eq!(
            query.get("client_id"),
            Some(&claude::CLAUDE_OAUTH_CLIENT_ID.into())
        );
        assert_eq!(
            query.get("redirect_uri"),
            Some(&claude::CLAUDE_OAUTH_CALLBACK_URL.into())
        );
        assert_eq!(query.get("scope"), Some(&claude::CLAUDE_OAUTH_SCOPE.into()));
        assert_eq!(query.get("code"), Some(&"true".into()));
        assert_eq!(query.get("state"), Some(&"state-value".into()));
        assert_eq!(query.get("code_challenge_method"), Some(&"S256".into()));
    }

    #[test]
    fn manual_callback_requires_the_pending_target_and_state() {
        let pending = PendingOAuth {
            product: AccountProduct::Codex,
            alias: String::new(),
            activate: false,
            code_verifier: "verifier".to_string(),
            state: "expected-state".to_string(),
            callback_url: OAUTH_CALLBACK_URL.to_string(),
            exchanging: false,
        };

        assert!(validate_callback_url(
            "http://localhost:1455/auth/callback?code=code&state=expected-state",
            &pending,
        )
        .is_ok());
        assert!(validate_callback_url(
            "http://localhost:1455/auth/callback?code=code&state=old-state",
            &pending,
        )
        .unwrap_err()
        .contains("state 不匹配"));
        assert!(validate_callback_url(
            "http://example.com:1455/auth/callback?code=code&state=expected-state",
            &pending,
        )
        .unwrap_err()
        .contains("不受信任"));
    }

    #[test]
    fn grok_browser_link_uses_the_pending_device_url() {
        let pending = PendingOAuth {
            product: AccountProduct::Grok,
            alias: String::new(),
            activate: false,
            code_verifier: String::new(),
            state: String::new(),
            callback_url: "https://auth.x.ai/device?code=ABCD".to_string(),
            exchanging: false,
        };

        assert_eq!(
            pending_authorization_url(&pending).unwrap(),
            pending.callback_url
        );
    }
}
