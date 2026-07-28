use super::{codex_auth::*, profiles::*, reset_credits::*, usage::*, *};
use base64::Engine;

fn oauth_auth(account_id: &str, user_id: &str, refresh_token: &str, access_token: &str) -> String {
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

fn access_token_auth(expires_at: Option<i64>) -> String {
    let encode = |value: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
    let claims = expires_at.map_or_else(|| json!({}), |exp| json!({ "exp": exp }));
    let access_token = format!(
        "{}.{}.{}",
        encode(br#"{"alg":"none","typ":"JWT"}"#),
        encode(claims.to_string().as_bytes()),
        encode(b"signature")
    );
    json!({ "tokens": { "access_token": access_token } }).to_string()
}

#[test]
fn prepares_temporary_codex_cli_credentials() {
    assert!(!codex_token_needs_refresh(&access_token_auth(Some(4_601)), 1_000).unwrap());
    assert!(codex_token_needs_refresh(&access_token_auth(Some(4_600)), 1_000).unwrap());
    assert!(!codex_token_needs_refresh(&access_token_auth(None), 1_000).unwrap());
    assert!(codex_token_needs_refresh(r#"{"tokens":{}}"#, 1_000).unwrap());
    let current = access_token_auth(None);
    assert!(codex_auth_needs_refresh(&current, Some(&current), 1_000).unwrap());
    assert!(!codex_auth_needs_refresh(&current, Some("already-refreshed"), 1_000).unwrap());

    let secret = "sk-secret-value";
    let (environment, arguments) =
        codex_relay_cli_options(secret.to_string(), "https://relay.example/v1");
    assert_eq!(
        environment,
        vec![(CODEX_RELAY_API_KEY_ENV.to_string(), secret.to_string())]
    );
    assert!(!arguments.join(" ").contains(secret));
    assert!(arguments
        .iter()
        .any(|argument| argument == "model_provider=\"cortana_relay\""));
    assert!(arguments
        .iter()
        .any(|argument| argument.contains("https://relay.example/v1")));
}

#[test]
fn keeps_refresh_intervals_independent_and_throttles_codex_requests() {
    let directory = std::env::temp_dir().join(format!("cortana-refresh-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let state = AppState {
        database_path: directory.join("app.sqlite3"),
        default_codex_home: directory.clone(),
        pending_oauth: Arc::new(Mutex::new(None)),
    };
    initialize_database(&state).unwrap();
    let mut connection = open_database(&state).unwrap();
    db::set_usage_refresh_settings(&mut connection, true, 10, 5).unwrap();
    assert_eq!(
        usage_refresh_settings(&state)
            .unwrap()
            .active_interval_minutes,
        10
    );
    assert_eq!(
        usage_refresh_settings(&state)
            .unwrap()
            .inactive_interval_minutes,
        5
    );

    connection
            .execute(
                "INSERT INTO accounts (id, product, account_type, alias, auth_json, created_at, updated_at)
                 VALUES ('oauth', 'codex', 'oauth', 'OAuth', '{}', 1, 1),
                        ('relay', 'codex', 'relay', 'Relay', '{}', 1, 1)",
                [],
            )
            .unwrap();
    let now = 1_000_000;
    assert!(claim_codex_usage_refresh(&connection, "oauth", now).unwrap());
    assert!(!claim_codex_usage_refresh(&connection, "oauth", now + 9_999).unwrap());
    assert!(claim_codex_usage_refresh(&connection, "oauth", now + 10_000).unwrap());
    assert!(!claim_codex_usage_refresh(&connection, "relay", now).unwrap());

    let settings = usage_refresh_settings(&state).unwrap();
    assert!(!usage_refresh_due(
        now,
        now - 599_999,
        true,
        settings,
        false
    ));
    assert!(usage_refresh_due(now, now - 600_000, true, settings, false));
    assert!(usage_refresh_due(
        now,
        now - 300_000,
        false,
        settings,
        false
    ));
    assert!(!usage_refresh_due(now, now - 9_999, false, settings, true));
    assert!(usage_refresh_due(now, now - 10_000, false, settings, true));
    connection
        .execute(
            "UPDATE accounts SET oauth_invalidated_at = ?1 WHERE id = 'oauth'",
            params![now],
        )
        .unwrap();
    assert!(
        get_profile_summary(&connection, "oauth", None)
            .unwrap()
            .needs_reauthorization
    );
    assert!(due_codex_profile_ids(&state, settings, true)
        .unwrap()
        .is_empty());

    drop(connection);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn isolates_accounts_by_product() {
    let directory = std::env::temp_dir().join(format!("cortana-product-test-{}", Uuid::new_v4()));
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
fn builds_reset_credit_http_request() {
    let encode = |value: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
    let claims = json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "account-from-token",
            "chatgpt_account_is_fedramp": true
        }
    });
    let id_token = format!(
        "{}.{}.{}",
        encode(br#"{"alg":"none","typ":"JWT"}"#),
        encode(claims.to_string().as_bytes()),
        encode(b"signature")
    );
    let auth_json = json!({
        "tokens": {
            "id_token": id_token,
            "access_token": "access-token"
        }
    })
    .to_string();
    let request = build_reset_credit_request(
        &Client::new(),
        reqwest::Method::POST,
        RESET_CREDITS_CONSUME_URL,
        &auth_json,
        "",
    )
    .unwrap()
    .build()
    .unwrap();

    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(request.headers()["Authorization"], "Bearer access-token");
    assert_eq!(
        request.headers()["ChatGPT-Account-ID"],
        "account-from-token"
    );
    assert_eq!(request.headers()["X-OpenAI-Fedramp"], "true");
    assert_eq!(request.headers()["User-Agent"], "codex-cli");
    let normal_request = build_reset_credit_request(
        &Client::new(),
        reqwest::Method::GET,
        RESET_CREDITS_URL,
        &oauth_auth("account", "user", "refresh-token", "access-token"),
        "account",
    )
    .unwrap()
    .build()
    .unwrap();
    assert!(normal_request.headers().get("X-OpenAI-Fedramp").is_none());
    assert_eq!(
        serde_json::to_value(ResetCreditConsumeRequest {
            redeem_request_id: "request-id",
            credit_id: "credit-id",
        })
        .unwrap(),
        json!({
            "redeem_request_id": "request-id",
            "credit_id": "credit-id"
        })
    );
}

#[test]
fn updates_profile_alias_auth_and_active_file() {
    let directory = std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
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

    let updated = update_profile_internal(&state, &profile.id, "新名称", &updated_auth).unwrap();

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
    let directory = std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
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
        upsert_relay_profile(&state, "relay-key", "https://other.example.com/v1", "Other").unwrap();
    assert_eq!(relay.id, duplicate.id);
    assert_ne!(relay.id, other.id);

    switch_profile_internal(&state, &relay.id, true).unwrap();
    let auth: Value =
        serde_json::from_str(&fs::read_to_string(directory.join("auth.json")).unwrap()).unwrap();
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
