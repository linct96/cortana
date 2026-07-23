use super::{accounts::*, antigravity, claude, codex::*, grok_oauth, tray::*, *};

enum OAuthExchange {
    Standard(OAuthTokenResponse),
    Claude(claude::ClaudeOAuthTokenResponse),
}

#[tauri::command]
pub(super) async fn import_auth_json(
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

pub(super) fn import_auth_json_internal(
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

#[tauri::command]
pub(super) fn start_oauth_add(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    alias: Option<String>,
    activate: bool,
    product: Option<AccountProduct>,
) -> Result<(), String> {
    let product = product.unwrap_or_default();
    if product == AccountProduct::Grok {
        return grok_oauth::start_device_oauth_add(app, state, alias, activate);
    }
    let mut pending = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    if pending.is_some() {
        return Err("已有一个授权流程正在进行，请先在浏览器中完成或关闭它。".to_string());
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
    });
    drop(pending);

    emit_progress(
        &app,
        "browser_opening",
        &format!("正在打开浏览器进行 {} 授权。", product.display_name()),
        None,
    );
    if let Err(error) = app.opener().open_url(auth_url, None::<&str>) {
        clear_pending_oauth(&state);
        let message = format!("无法打开默认浏览器：{error}");
        emit_progress(&app, "error", &message, None);
        return Err(message);
    }

    let state_for_thread = state.inner().clone();
    thread::spawn(move || wait_for_oauth_callback(app, state_for_thread, listeners));
    Ok(())
}

#[tauri::command]
pub(super) fn cancel_oauth_add(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    grok_oauth::cancel_device_oauth();
    clear_pending_oauth(&state);
    emit_progress(&app, "cancelled", "授权已取消。", None);
    Ok(())
}

pub(super) fn build_authorize_url(
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

pub(super) fn bind_oauth_listeners(
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

pub(super) fn wait_for_oauth_callback(
    app: tauri::AppHandle,
    state: AppState,
    listeners: Vec<TcpListener>,
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
                    let pending_context = state.pending_oauth.lock().ok().and_then(|pending| {
                        pending
                            .as_ref()
                            .map(|value| (value.product, value.callback_url.clone()))
                    });
                    let callback = pending_context
                        .as_ref()
                        .map(|(_, callback_url)| callback_url)
                        .ok_or_else(|| "OAuth 授权已取消。".to_string())
                        .and_then(|callback_url| read_callback_url(&mut stream, callback_url));
                    match callback.and_then(|url| complete_oauth_callback(&app, &state, url)) {
                        Ok(profile) => {
                            let _ = write_browser_response(
                                &mut stream,
                                pending_context
                                    .map(|(product, _)| product)
                                    .unwrap_or_default(),
                                true,
                                "授权完成，可以返回 Cortana。 ",
                            );
                            emit_progress(&app, "success", "账户已添加。", Some(profile));
                        }
                        Err(message) => {
                            clear_pending_oauth(&state);
                            let _ = write_browser_response(
                                &mut stream,
                                pending_context
                                    .map(|(product, _)| product)
                                    .unwrap_or_default(),
                                false,
                                "授权未完成，可以回到应用重试。 ",
                            );
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

pub(super) fn read_callback_url(
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

pub(super) fn complete_oauth_callback(
    app: &tauri::AppHandle,
    state: &AppState,
    callback_url: String,
) -> Result<ProfileSummary, String> {
    let callback = Url::parse(&callback_url).map_err(|_| "OAuth 回调地址无效。".to_string())?;
    let received_state = callback
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| "OAuth 回调缺少 state。".to_string())?;
    let pending_guard = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    let pending_callback = pending_guard
        .as_ref()
        .ok_or_else(|| "OAuth 授权状态不存在或已过期。".to_string())?;
    let expected_callback = Url::parse(&pending_callback.callback_url)
        .map_err(|_| "OAuth 回调地址无效。".to_string())?;
    if callback.host_str() != expected_callback.host_str()
        || callback.port_or_known_default() != expected_callback.port_or_known_default()
        || callback.path() != expected_callback.path()
    {
        return Err("OAuth 回调地址不受信任。".to_string());
    }
    let matches_pending_state = pending_callback.state == received_state;
    if !matches_pending_state {
        return Err("OAuth state 不匹配，已拒绝本次回调。".to_string());
    }
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
    emit_progress(app, "exchanging", "正在交换授权信息。", None);
    let exchange = match pending.product {
        AccountProduct::Claude => OAuthExchange::Claude(claude::exchange_code(
            &code,
            &pending.state,
            &pending.code_verifier,
            &pending.callback_url,
        )?),
        AccountProduct::Codex | AccountProduct::Antigravity => {
            OAuthExchange::Standard(exchange_code(
                pending.product,
                &code,
                &pending.code_verifier,
                &pending.callback_url,
            )?)
        }
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

pub(super) fn refresh_oauth_token(refresh_token: &str) -> Result<OAuthTokenResponse, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(OAUTH_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", OAUTH_CLIENT_ID),
        ])
        .send()
        .map_err(|error| format!("认证信息刷新请求失败：{error}"))?;
    let status = response.status();
    let body = response.text().map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!("认证信息刷新失败（HTTP {}）。", status.as_u16()));
    }
    let mut token: OAuthTokenResponse =
        serde_json::from_str(&body).map_err(|_| "认证信息刷新响应不是有效 JSON。".to_string())?;
    if token.access_token.as_deref().unwrap_or_default().is_empty() {
        return Err("认证信息刷新未返回 access_token。".to_string());
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

pub(super) fn build_codex_auth_json(token: &OAuthTokenResponse) -> Result<String, String> {
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
        "OPENAI_API_KEY": Value::Null,
        "tokens": {
            "access_token": access_token,
            "refresh_token": token.refresh_token.as_deref().unwrap_or_default(),
            "id_token": token.id_token.as_deref().unwrap_or_default(),
            "account_id": identity.account_id,
        },
        "last_refresh": Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true),
    }))
    .map_err(|error| error.to_string())
}

pub(super) fn identity_from_auth_json(auth: &Value) -> Identity {
    let tokens = auth.get("tokens").and_then(Value::as_object);
    let id_token = tokens
        .and_then(|tokens| tokens.get("id_token"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut identity = identity_from_id_token(id_token);
    if identity.account_id.is_empty() {
        identity.account_id = tokens
            .and_then(|tokens| tokens.get("account_id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    let access_token = tokens
        .and_then(|tokens| tokens.get("access_token"))
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

pub(super) fn identity_from_id_token(token: &str) -> Identity {
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

pub(super) fn identity_from_jwt(token: &str) -> Identity {
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

pub(super) fn decode_jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&decoded).ok()
}

pub(super) fn random_urlsafe(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    rand::thread_rng().fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

pub(super) fn exchange_code(
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

pub(super) fn clear_pending_oauth(state: &AppState) {
    if let Ok(mut pending) = state.pending_oauth.lock() {
        *pending = None;
    }
}

pub(super) fn emit_progress(
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

pub(super) fn write_browser_response(
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
    use super::*;

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
    fn antigravity_callback_listens_on_every_available_loopback_stack() {
        let (listeners, callback) = bind_oauth_listeners(AccountProduct::Antigravity).unwrap();
        let url = Url::parse(&callback).unwrap();

        assert!(!listeners.is_empty());
        assert_eq!(url.path(), "/auth/callback");
        if listeners.len() == 2 {
            assert_eq!(url.host_str(), Some("localhost"));
        }
    }
    #[test]
    fn builds_refreshed_auth_from_an_access_token_without_an_id_token() {
        let claims = json!({
            "email": "person@example.com",
            "https://api.openai.com/auth": { "chatgpt_account_id": "account-123" }
        });
        let access_token = format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let token = OAuthTokenResponse {
            access_token: Some(access_token),
            refresh_token: Some("refreshed-rt".to_string()),
            id_token: None,
            expires_in: None,
            token_type: None,
        };

        let auth: Value = serde_json::from_str(&build_codex_auth_json(&token).unwrap()).unwrap();

        assert_eq!(auth["tokens"]["account_id"], "account-123");
        assert_eq!(auth["tokens"]["refresh_token"], "refreshed-rt");
    }

    #[test]
    fn reads_name_only_from_id_token() {
        let token = |claims: Value| {
            format!(
                "header.{}.signature",
                URL_SAFE_NO_PAD.encode(claims.to_string())
            )
        };
        let auth = json!({
            "tokens": {
                "id_token": token(json!({ "name": "ID Token Name" })),
                "access_token": token(json!({ "name": "Access Token Name" }))
            }
        });

        assert_eq!(identity_from_auth_json(&auth).name, "ID Token Name");
    }
}
