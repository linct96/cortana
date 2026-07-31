use super::{
    auth::{
        authentication_invalidated, backend_error_message, with_codex_auth_retry, CodexApiError,
    },
    usage::refresh_profile_usage_with_credits_internal,
};
use crate::{
    features::accounts::oauth::{decode_jwt_claims, identity_from_auth_json},
    platform::{
        db::{database_error, open_database},
        state::{
            AppState, ResetCredit, ResetCreditConsumeOutcome, ResetCreditConsumeResult,
            ResetCredits, RESET_CREDITS_CONSUME_URL, RESET_CREDITS_URL,
        },
    },
};
use reqwest::blocking::Client;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tauri::State;
use uuid::Uuid;

pub(crate) async fn get_profile_reset_credits(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<ResetCredits, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let credits = with_codex_auth_retry(&state, &profile_id, |auth| {
            fetch_reset_credits(&auth.auth_json, &auth.account_id)
        })?;
        let connection = open_database(&state)?;
        connection
            .execute(
                "UPDATE accounts SET reset_credits_available_count = ?1 WHERE id = ?2 AND product = 'codex'",
                params![credits.available_count, profile_id],
            )
            .map_err(database_error)?;
        Ok(credits)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) async fn consume_profile_reset_credit(
    state: State<'_, AppState>,
    profile_id: String,
    credit_id: String,
    idempotency_key: String,
) -> Result<ResetCreditConsumeResult, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        consume_profile_reset_credit_internal(&state, &profile_id, &credit_id, &idempotency_key)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn consume_profile_reset_credit_internal(
    state: &AppState,
    profile_id: &str,
    credit_id: &str,
    idempotency_key: &str,
) -> Result<ResetCreditConsumeResult, String> {
    let (credit_id, idempotency_key) = validate_reset_credit_request(credit_id, idempotency_key)?;
    let outcome = with_codex_auth_retry(state, profile_id, |auth| {
        consume_reset_credit(
            &auth.auth_json,
            &auth.account_id,
            &credit_id,
            &idempotency_key,
        )
    })?;
    let (profile, credits) = refresh_profile_usage_with_credits_internal(state, profile_id)?;
    let credits = match credits {
        Some(credits) => credits,
        None => with_codex_auth_retry(state, profile_id, |auth| {
            fetch_reset_credits(&auth.auth_json, &auth.account_id)
        })?,
    };
    Ok(ResetCreditConsumeResult {
        outcome,
        profile,
        credits,
    })
}

pub(crate) fn validate_reset_credit_request(
    credit_id: &str,
    idempotency_key: &str,
) -> Result<(String, String), String> {
    let credit_id = credit_id.trim();
    if credit_id.is_empty() {
        return Err("重置卡 ID 不能为空。".to_string());
    }
    let idempotency_key = Uuid::parse_str(idempotency_key.trim())
        .map_err(|_| "幂等键必须是有效的 UUID。".to_string())?
        .to_string();
    Ok((credit_id.to_string(), idempotency_key))
}

#[derive(Serialize)]
pub(crate) struct ResetCreditConsumeRequest<'a> {
    pub(crate) redeem_request_id: &'a str,
    pub(crate) credit_id: &'a str,
}

pub(crate) fn consume_reset_credit(
    auth_json: &str,
    account_id: &str,
    credit_id: &str,
    idempotency_key: &str,
) -> Result<ResetCreditConsumeOutcome, CodexApiError> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| CodexApiError {
            message: error.to_string(),
            unauthorized: false,
            authentication_invalidated: false,
        })?;
    let request = build_reset_credit_request(
        &client,
        reqwest::Method::POST,
        RESET_CREDITS_CONSUME_URL,
        auth_json,
        account_id,
    )
    .map_err(|message| CodexApiError {
        message,
        unauthorized: false,
        authentication_invalidated: false,
    })?;
    let response = request
        .json(&ResetCreditConsumeRequest {
            redeem_request_id: idempotency_key,
            credit_id,
        })
        .send()
        .map_err(|error| CodexApiError {
            message: format!("重置卡使用失败：{error}"),
            unauthorized: false,
            authentication_invalidated: false,
        })?;
    let status = response.status();
    let body = response.text().map_err(|error| CodexApiError {
        message: format!("无法读取重置卡使用结果：{error}"),
        unauthorized: false,
        authentication_invalidated: false,
    })?;
    if !status.is_success() {
        let message = backend_error_message(&body);
        return Err(CodexApiError {
            message: if message.is_empty() {
                format!("重置卡使用失败：HTTP {status}")
            } else {
                format!("重置卡使用失败：{message}")
            },
            unauthorized: status.as_u16() == 401,
            authentication_invalidated: authentication_invalidated(status.as_u16(), &body),
        });
    }
    parse_reset_credit_consume_response(&body).map_err(|message| CodexApiError {
        message,
        unauthorized: false,
        authentication_invalidated: false,
    })
}

pub(crate) fn parse_reset_credit_consume_response(
    body: &str,
) -> Result<ResetCreditConsumeOutcome, String> {
    let payload: Value = serde_json::from_str(body)
        .map_err(|error| format!("重置卡使用响应格式不符合预期：{error}"))?;
    match payload.get("code").and_then(Value::as_str) {
        Some("reset") => Ok(ResetCreditConsumeOutcome::Reset),
        Some("already_redeemed") => Ok(ResetCreditConsumeOutcome::AlreadyRedeemed),
        Some("nothing_to_reset") => Ok(ResetCreditConsumeOutcome::NothingToReset),
        Some("no_credit") => Ok(ResetCreditConsumeOutcome::NoCredit),
        Some(_) => Err("重置卡使用接口返回了未知结果。".to_string()),
        None => Err("重置卡使用响应缺少 code。".to_string()),
    }
}

#[derive(Deserialize)]
pub(crate) struct ResetCreditsResponse {
    available_count: i64,
    credits: Vec<ResetCreditResponse>,
}

#[derive(Deserialize)]
pub(crate) struct ResetCreditResponse {
    id: String,
    title: String,
    status: String,
    expires_at: String,
    granted_at: String,
}

pub(crate) fn fetch_reset_credits(
    auth_json: &str,
    account_id: &str,
) -> Result<ResetCredits, CodexApiError> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| CodexApiError {
            message: error.to_string(),
            unauthorized: false,
            authentication_invalidated: false,
        })?;
    let response = build_reset_credit_request(
        &client,
        reqwest::Method::GET,
        RESET_CREDITS_URL,
        auth_json,
        account_id,
    )
    .map_err(|message| CodexApiError {
        message,
        unauthorized: false,
        authentication_invalidated: false,
    })?
    .send()
    .map_err(|error| CodexApiError {
        message: format!("重置卡查询失败：{error}"),
        unauthorized: false,
        authentication_invalidated: false,
    })?;
    let status = response.status();
    let body = response.text().map_err(|error| CodexApiError {
        message: format!("无法读取重置卡信息：{error}"),
        unauthorized: false,
        authentication_invalidated: false,
    })?;
    if !status.is_success() {
        return Err(CodexApiError {
            message: format!("重置卡查询失败：HTTP {status}"),
            unauthorized: status.as_u16() == 401,
            authentication_invalidated: authentication_invalidated(status.as_u16(), &body),
        });
    }
    parse_reset_credits(&body).map_err(|message| CodexApiError {
        message,
        unauthorized: false,
        authentication_invalidated: false,
    })
}

pub(crate) fn build_reset_credit_request(
    client: &Client,
    method: reqwest::Method,
    url: &str,
    auth_json: &str,
    account_id: &str,
) -> Result<reqwest::blocking::RequestBuilder, String> {
    let auth: Value =
        serde_json::from_str(auth_json).map_err(|_| "存档的 auth.json 已损坏。".to_string())?;
    let tokens = auth.get("tokens").and_then(Value::as_object);
    let access_token = tokens
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
    let mut request = client
        .request(method, url)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("User-Agent", "codex-cli");
    if !account_id.is_empty() {
        request = request.header("ChatGPT-Account-ID", account_id);
    }
    let fedramp = ["id_token", "access_token"].into_iter().any(|key| {
        tokens
            .and_then(|tokens| tokens.get(key))
            .and_then(Value::as_str)
            .and_then(decode_jwt_claims)
            .and_then(|claims| {
                claims
                    .get("https://api.openai.com/auth")
                    .and_then(|auth| auth.get("chatgpt_account_is_fedramp"))
                    .and_then(Value::as_bool)
            })
            .unwrap_or(false)
    });
    if fedramp {
        request = request.header("X-OpenAI-Fedramp", "true");
    }
    Ok(request)
}

pub(crate) fn parse_reset_credits(body: &str) -> Result<ResetCredits, String> {
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
