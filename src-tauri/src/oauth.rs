use super::{accounts::*, codex::*, tray::*, *};

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
        false,
    )?;
    if activate {
        switch_profile_internal(state, &profile.id, true)?;
    }
    refresh_tray(app)?;
    Ok(profile)
}

#[tauri::command]
pub(super) fn start_oauth_add(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    alias: Option<String>,
    activate: bool,
) -> Result<(), String> {
    let mut pending = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    if pending.is_some() {
        return Err("已有一个授权流程正在进行，请先在浏览器中完成或关闭它。".to_string());
    }

    let listeners = bind_oauth_listeners()?;
    let code_verifier = random_urlsafe(32);
    let state_token = random_urlsafe(32);
    let auth_url = build_authorize_url(&code_verifier, &state_token)?;
    *pending = Some(PendingOAuth {
        alias: alias.unwrap_or_default().trim().to_string(),
        activate,
        code_verifier,
        state: state_token,
    });
    drop(pending);

    emit_progress(
        &app,
        "browser_opening",
        "正在打开浏览器进行 Codex 授权。",
        None,
    );
    if let Err(error) = app.opener().open_url(auth_url, None::<&str>) {
        clear_pending_oauth(&state);
        return Err(format!("无法打开默认浏览器：{error}"));
    }

    let state_for_thread = state.inner().clone();
    thread::spawn(move || wait_for_oauth_callback(app, state_for_thread, listeners));
    Ok(())
}

#[tauri::command]
pub(super) fn cancel_oauth_add(state: State<'_, AppState>) -> Result<(), String> {
    clear_pending_oauth(&state);
    Ok(())
}

pub(super) fn build_authorize_url(code_verifier: &str, state: &str) -> Result<String, String> {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
    let mut url = Url::parse(OAUTH_AUTHORIZE_URL).map_err(|error| error.to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", OAUTH_CLIENT_ID);
        query.append_pair("redirect_uri", OAUTH_CALLBACK_URL);
        query.append_pair("scope", OAUTH_SCOPE);
        query.append_pair("code_challenge", &challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("id_token_add_organizations", "true");
        query.append_pair("codex_cli_simplified_flow", "true");
        query.append_pair("state", state);
        query.append_pair("originator", "codex_cli_rs");
    }
    Ok(url.into())
}

pub(super) fn bind_oauth_listeners() -> Result<Vec<TcpListener>, String> {
    let mut listeners = Vec::new();
    for address in ["127.0.0.1:1455", "[::1]:1455"] {
        match TcpListener::bind(address) {
            Ok(listener) => {
                listener
                    .set_nonblocking(true)
                    .map_err(|error| error.to_string())?;
                listeners.push(listener);
            }
            Err(_) => {}
        }
    }
    if listeners.is_empty() {
        return Err("无法监听 localhost:1455。请关闭占用该端口的程序后重试。".to_string());
    }
    Ok(listeners)
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
                    let callback = read_callback_url(&mut stream);
                    match callback.and_then(|url| complete_oauth_callback(&app, &state, url)) {
                        Ok(profile) => {
                            let _ = write_browser_response(
                                &mut stream,
                                true,
                                "授权完成，可以返回 Cortana。 ",
                            );
                            emit_progress(&app, "success", "账户已添加。", Some(profile));
                        }
                        Err(message) => {
                            let _ = write_browser_response(
                                &mut stream,
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

pub(super) fn read_callback_url(stream: &mut TcpStream) -> Result<String, String> {
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
    Ok(format!("http://localhost:1455{target}"))
}

pub(super) fn complete_oauth_callback(
    app: &tauri::AppHandle,
    state: &AppState,
    callback_url: String,
) -> Result<ProfileSummary, String> {
    let callback = Url::parse(&callback_url).map_err(|_| "OAuth 回调地址无效。".to_string())?;
    if callback.host_str() != Some("localhost")
        || callback.port() != Some(1455)
        || callback.path() != "/auth/callback"
    {
        return Err("OAuth 回调地址不受信任。".to_string());
    }
    let error = callback
        .query_pairs()
        .find(|(key, _)| key == "error")
        .map(|(_, value)| value.into_owned());
    if let Some(error) = error {
        clear_pending_oauth(state);
        return Err(format!("OAuth 授权被拒绝：{error}"));
    }
    let code = callback
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| "OAuth 回调缺少授权 code。".to_string())?;
    let received_state = callback
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| "OAuth 回调缺少 state。".to_string())?;
    let pending_guard = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    let matches_pending_state = pending_guard
        .as_ref()
        .map(|pending| pending.state == received_state)
        .ok_or_else(|| "OAuth 授权状态不存在或已过期。".to_string())?;
    if !matches_pending_state {
        return Err("OAuth state 不匹配，已拒绝本次回调。".to_string());
    }
    let pending = pending_guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "OAuth 授权状态不存在或已过期。".to_string())?;
    drop(pending_guard);
    emit_progress(app, "exchanging", "正在交换授权信息。", None);
    let token = exchange_code(&code, &pending.code_verifier)?;
    let auth_json = build_codex_auth_json(&token)?;
    let mut pending_guard = state
        .pending_oauth
        .lock()
        .map_err(|_| "OAuth 状态锁不可用。".to_string())?;
    if pending_guard.as_ref().map(|current| current.state.as_str()) != Some(&pending.state) {
        return Err("OAuth 授权已取消。".to_string());
    }
    pending_guard.take();
    drop(pending_guard);
    let profile = upsert_profile_from_auth(state, &auth_json, &pending.alias, false)?;
    if pending.activate {
        switch_profile_internal(state, &profile.id, true)?;
    }
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
        .map(identity_from_jwt)
        .unwrap_or_default();
    if identity.account_id.is_empty() {
        identity = identity_from_jwt(access_token);
    }
    Ok(serde_json::to_string_pretty(&json!({
        "OPENAI_API_KEY": Value::Null,
        "tokens": {
            "access_token": access_token,
            "refresh_token": token.refresh_token.as_deref().unwrap_or_default(),
            "id_token": token.id_token.as_deref().unwrap_or_default(),
            "account_id": identity.account_id,
        },
        "last_refresh": Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true),
    }))
    .map_err(|error| error.to_string())?)
}

pub(super) fn identity_from_auth_json(auth: &Value) -> Identity {
    let tokens = auth.get("tokens").and_then(Value::as_object);
    let id_token = tokens
        .and_then(|tokens| tokens.get("id_token"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut identity = identity_from_jwt(id_token);
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

pub(super) fn exchange_code(code: &str, verifier: &str) -> Result<OAuthTokenResponse, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(OAUTH_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", OAUTH_CALLBACK_URL),
            ("client_id", OAUTH_CLIENT_ID),
            ("code_verifier", verifier),
        ])
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
    let _ = app.emit(
        "oauth-progress",
        OAuthProgress {
            stage: stage.to_string(),
            message: message.to_string(),
            profile,
        },
    );
}

pub(super) fn write_browser_response(
    stream: &mut TcpStream,
    success: bool,
    message: &str,
) -> std::io::Result<()> {
    let title = if success {
        "Codex 授权完成"
    } else {
        "Codex 授权未完成"
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
        let url = Url::parse(&build_authorize_url("verifier", "state-value").unwrap()).unwrap();
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
    fn oauth_cancellation_clears_the_pending_authorization() {
        let state = AppState {
            database_path: PathBuf::new(),
            default_codex_home: PathBuf::new(),
            pending_oauth: Arc::new(Mutex::new(Some(PendingOAuth {
                alias: String::new(),
                activate: false,
                code_verifier: "verifier".to_string(),
                state: "state".to_string(),
            }))),
        };

        clear_pending_oauth(&state);

        assert!(state.pending_oauth.lock().unwrap().is_none());
    }

    #[test]
    fn extracts_identity_from_codex_tokens() {
        let claims = json!({
            "email": "person@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "account-123",
                "chatgpt_plan_type": "plus"
            }
        });
        let token = format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let auth = json!({ "tokens": { "id_token": token } });

        let identity = identity_from_auth_json(&auth);

        assert_eq!(identity.email, "person@example.com");
        assert_eq!(identity.account_id, "account-123");
        assert_eq!(identity.plan_type, "plus");
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
        };

        let auth: Value = serde_json::from_str(&build_codex_auth_json(&token).unwrap()).unwrap();

        assert_eq!(auth["tokens"]["account_id"], "account-123");
        assert_eq!(auth["tokens"]["refresh_token"], "refreshed-rt");
    }
}
