use super::{
    is_enabled, local_api_key,
    logging::{GatewayLogger, ObservedReader, RequestTrace},
    openai_responses, set_available, UpstreamAuthMode, UpstreamProtocol, ACTIVE_PROFILE_SETTING,
    GATEWAY_PORT,
};
use crate::{
    platform::{
        db::{credential_fingerprint, database_error, get_setting, open_database},
        state::{AppState, ACCOUNT_TYPE_OAUTH, ACCOUNT_TYPE_RELAY},
    },
    products::codex::{
        auth::{
            authentication_invalidated, backend_error_message, codex_access_token,
            with_codex_auth_retry, CodexApiError,
        },
        extract_api_key,
    },
};
use reqwest::blocking::{Client, RequestBuilder, Response as UpstreamResponse};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{cell::Cell, io::Read, sync::Arc, thread, time::Duration, time::Instant};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::Url;

const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 1024 * 1024;
const WORKER_COUNT: usize = 8;
const FORWARDED_REQUEST_HEADERS: &[&str] = &[
    "User-Agent",
    "originator",
    "version",
    "session-id",
    "thread-id",
    "x-client-request-id",
    "x-codex-beta-features",
    "x-codex-installation-id",
    "x-codex-turn-metadata",
    "x-codex-turn-state",
    "x-codex-window-id",
    "x-codex-parent-thread-id",
    "x-openai-memgen-request",
    "x-openai-subagent",
    "x-oai-attestation",
    "x-responsesapi-include-timing-metrics",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GatewayEndpoint {
    Responses,
    Compact,
}

impl GatewayEndpoint {
    fn parse(path: &str) -> Option<Self> {
        match path {
            "/v1/responses" => Some(Self::Responses),
            "/v1/responses/compact" => Some(Self::Compact),
            _ => None,
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::Compact => "responses/compact",
        }
    }

    fn accept(self) -> &'static str {
        match self {
            Self::Responses => "text/event-stream",
            Self::Compact => "application/json",
        }
    }

    fn forwards_header(self, name: &str) -> bool {
        self == Self::Responses || !name.eq_ignore_ascii_case("x-client-request-id")
    }

    fn requires_response_media_type(self) -> bool {
        self == Self::Compact
    }
}

enum GatewayAccount {
    OAuth {
        profile_id: String,
        alias: String,
        email: String,
    },
    Relay {
        alias: String,
        email: String,
        base_url: String,
        api_key: String,
        protocol: UpstreamProtocol,
        auth_mode: UpstreamAuthMode,
        anthropic_max_tokens: i64,
    },
}

impl GatewayAccount {
    fn log_identity(&self) -> (&str, &str, &str, &str) {
        match self {
            Self::OAuth { alias, email, .. } => (
                alias,
                email,
                ACCOUNT_TYPE_OAUTH,
                UpstreamProtocol::OpenAiResponses.as_str(),
            ),
            Self::Relay {
                alias,
                email,
                protocol,
                ..
            } => (alias, email, ACCOUNT_TYPE_RELAY, protocol.as_str()),
        }
    }
}

pub(crate) fn initialize(state: AppState) -> Result<(), String> {
    let data_dir = state
        .database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let logger = GatewayLogger::initialize(data_dir);
    let server = match Server::http(("127.0.0.1", GATEWAY_PORT)) {
        Ok(server) => Arc::new(server),
        Err(error) => {
            set_available(false);
            logger.system_event(
                "error",
                "gateway.bind_failed",
                json!({"address":format!("127.0.0.1:{GATEWAY_PORT}"),"message":error.to_string()}),
            );
            eprintln!("Model gateway unavailable on 127.0.0.1:{GATEWAY_PORT}: {error}");
            return Ok(());
        }
    };
    let client = Arc::new(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(5 * 60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| format!("无法创建协议转换客户端：{error}"))?,
    );
    for _ in 0..WORKER_COUNT {
        let server = server.clone();
        let client = client.clone();
        let state = state.clone();
        let logger = logger.clone();
        thread::spawn(move || loop {
            match server.recv() {
                Ok(request) => handle_request(request, &state, &client, &logger),
                Err(error) => {
                    logger.system_event(
                        "error",
                        "gateway.receive_failed",
                        json!({"message":error.to_string()}),
                    );
                    eprintln!("Model gateway request failed: {error}");
                    break;
                }
            }
        });
    }
    logger.system_event(
        "info",
        "gateway.started",
        json!({"address":format!("127.0.0.1:{GATEWAY_PORT}"),"workers":WORKER_COUNT}),
    );
    set_available(true);
    Ok(())
}

fn handle_request(
    mut request: Request,
    state: &AppState,
    client: &Client,
    logger: &Arc<GatewayLogger>,
) {
    let session_id = header(&request, "session-id");
    let mut trace = RequestTrace::new(
        logger.clone(),
        request.method().as_str(),
        request.url(),
        header(&request, "x-client-request-id"),
        session_id.as_deref(),
    );
    trace.event(
        "info",
        "request.received",
        json!({
            "content_type":header(&request,"Content-Type"),
            "declared_body_bytes":request.body_length(),
        }),
    );
    if !is_local_host(header(&request, "Host").as_deref().unwrap_or_default()) {
        respond_error(request, &trace, 403, "invalid_request", "请求来源无效。");
        return;
    }
    if header(&request, "Origin").is_some() {
        respond_error(
            request,
            &trace,
            403,
            "invalid_request",
            "不接受浏览器 Origin 请求。",
        );
        return;
    }
    if request.method() != &Method::Post {
        respond_error(
            request,
            &trace,
            405,
            "invalid_request",
            "仅支持 POST 请求。",
        );
        return;
    }
    let Some(endpoint) = GatewayEndpoint::parse(request.url()) else {
        respond_error(request, &trace, 404, "invalid_request", "网关路径不存在。");
        return;
    };
    if media_type(header(&request, "Content-Type").as_deref()) != Some("application/json") {
        respond_error(
            request,
            &trace,
            415,
            "invalid_request",
            "请求必须使用 application/json。",
        );
        return;
    }
    let authorization = header(&request, "Authorization");
    let Some(token) = bearer_token(authorization.as_deref()) else {
        respond_error(
            request,
            &trace,
            401,
            "invalid_api_key",
            "缺少 Bearer Token。",
        );
        return;
    };
    let account = match gateway_account(state, token) {
        Ok(account) => account,
        Err(error) => {
            respond_error(request, &trace, 401, "invalid_api_key", &error);
            return;
        }
    };
    let (alias, email, account_type, protocol) = account.log_identity();
    trace.set_account(alias, email, account_type, protocol);
    trace.event("info", "request.routed", json!({}));
    if request
        .body_length()
        .is_some_and(|size| size > MAX_REQUEST_BYTES)
    {
        respond_error(request, &trace, 413, "request_too_large", "请求数据过大。");
        return;
    }
    let forwarded_headers = forwarded_request_headers(&request, endpoint);
    let installation_id = header(&request, "x-codex-installation-id");
    let mut body = Vec::new();
    if request
        .as_reader()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .is_err()
    {
        respond_error(
            request,
            &trace,
            400,
            "invalid_request",
            "无法读取请求数据。",
        );
        return;
    }
    if body.len() > MAX_REQUEST_BYTES {
        respond_error(request, &trace, 413, "request_too_large", "请求数据过大。");
        return;
    }
    let input: Value = match serde_json::from_slice::<Value>(&body) {
        Ok(value) if value.is_object() => value,
        _ => {
            respond_error(request, &trace, 400, "invalid_json", "请求 JSON 格式无效。");
            return;
        }
    };
    trace.set_model(input.get("model").and_then(Value::as_str));
    trace.payload("request.body", "inbound", &input);
    trace.event(
        "info",
        "request.validated",
        json!({"body_bytes":body.len()}),
    );

    match account {
        GatewayAccount::OAuth { profile_id, .. } => {
            let input = match endpoint {
                GatewayEndpoint::Responses => openai_responses::prepare_codex_oauth_request(
                    input,
                    session_id.as_deref(),
                    installation_id.as_deref(),
                ),
                GatewayEndpoint::Compact => {
                    openai_responses::prepare_codex_compact_request(input, session_id.as_deref())
                }
            };
            let input = match input {
                Ok(input) => input,
                Err(error) => {
                    respond_error(request, &trace, 400, "invalid_request", &error);
                    return;
                }
            };
            trace.payload("upstream.request.body", "outbound", &input);
            let attempts = Cell::new(0_u32);
            let upstream = with_codex_auth_retry(state, &profile_id, |auth| {
                let attempt = attempts.get() + 1;
                attempts.set(attempt);
                let url = format!("{CODEX_BASE_URL}/{}", endpoint.suffix());
                trace.event(
                    "info",
                    "upstream.request",
                    json!({"attempt":attempt,"url":safe_upstream_url(&url)}),
                );
                let started = Instant::now();
                let access_token =
                    codex_access_token(&auth.auth_json).map_err(|message| CodexApiError {
                        message,
                        unauthorized: false,
                        authentication_invalidated: false,
                    })?;
                let request = client
                    .post(url)
                    .bearer_auth(access_token)
                    .header("Chatgpt-Account-Id", &auth.account_id)
                    .header("Content-Type", "application/json")
                    .header("Accept", endpoint.accept())
                    .json(&input);
                let request = apply_forwarded_headers(request, &forwarded_headers, true);
                let mut response = request.send().map_err(|error| {
                    trace.event(
                        "error",
                        "upstream.connection_failed",
                        json!({"attempt":attempt,"duration_ms":started.elapsed().as_millis(),"message":error.to_string()}),
                    );
                    CodexApiError {
                        message: format!("无法连接 Codex 上游：{error}"),
                        unauthorized: false,
                        authentication_invalidated: false,
                    }
                })?;
                log_upstream_response(&trace, &response, attempt, started.elapsed());
                if response.status().as_u16() != 401 {
                    return Ok(response);
                }
                let body = read_limited(&mut response, MAX_ERROR_BYTES).unwrap_or_default();
                log_upstream_error_body(&trace, &body);
                Err(CodexApiError {
                    message: backend_error_message(&String::from_utf8_lossy(&body)),
                    unauthorized: true,
                    authentication_invalidated: authentication_invalidated(
                        401,
                        &String::from_utf8_lossy(&body),
                    ),
                })
            });
            match upstream {
                Ok(upstream) => respond_native(request, upstream, endpoint, &trace),
                Err(error) => respond_error(request, &trace, 502, "upstream_error", &error),
            }
        }
        GatewayAccount::Relay {
            base_url,
            api_key,
            protocol,
            auth_mode,
            anthropic_max_tokens,
            ..
        } => {
            if protocol == UpstreamProtocol::OpenAiResponses {
                trace.payload("upstream.request.body", "outbound", &input);
                let url = upstream_url(&base_url, endpoint);
                trace.event(
                    "info",
                    "upstream.request",
                    json!({"attempt":1,"url":safe_upstream_url(&url)}),
                );
                let started = Instant::now();
                let request_builder = client
                    .post(url)
                    .header("Accept", endpoint.accept())
                    .json(&input);
                let request_builder = apply_upstream_auth(request_builder, auth_mode, &api_key);
                let request_builder =
                    apply_forwarded_headers(request_builder, &forwarded_headers, false);
                match request_builder.send() {
                    Ok(upstream) => {
                        log_upstream_response(&trace, &upstream, 1, started.elapsed());
                        respond_native(request, upstream, endpoint, &trace)
                    }
                    Err(error) => {
                        trace.event(
                            "error",
                            "upstream.connection_failed",
                            json!({"attempt":1,"duration_ms":started.elapsed().as_millis(),"message":error.to_string()}),
                        );
                        respond_error(
                            request,
                            &trace,
                            502,
                            "upstream_connection_error",
                            &format!("无法连接上游：{error}"),
                        )
                    }
                }
                return;
            }
            if endpoint == GatewayEndpoint::Compact {
                respond_error(
                    request,
                    &trace,
                    400,
                    "unsupported_feature",
                    "当前上游协议不支持 Responses Compact。",
                );
                return;
            }
            let encoded =
                match openai_responses::encode_request(protocol, input, anthropic_max_tokens) {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        respond_error(request, &trace, 400, "unsupported_feature", &error);
                        return;
                    }
                };
            trace.payload("upstream.request.body", "outbound", &encoded.body);
            let endpoint = match protocol {
                UpstreamProtocol::OpenAiChatCompletions => "chat/completions",
                UpstreamProtocol::AnthropicMessages => "messages",
                UpstreamProtocol::OpenAiResponses => unreachable!(),
            };
            let url = format!("{}/{endpoint}", base_url.trim_end_matches('/'));
            trace.event(
                "info",
                "upstream.request",
                json!({"attempt":1,"url":safe_upstream_url(&url)}),
            );
            let started = Instant::now();
            let request_builder = client
                .post(url)
                .header("Accept", "text/event-stream")
                .json(&encoded.body);
            let mut request_builder = apply_upstream_auth(request_builder, auth_mode, &api_key);
            if protocol == UpstreamProtocol::AnthropicMessages {
                request_builder = request_builder.header("anthropic-version", "2023-06-01");
            }
            let mut upstream = match request_builder.send() {
                Ok(response) => response,
                Err(error) => {
                    trace.event(
                        "error",
                        "upstream.connection_failed",
                        json!({"attempt":1,"duration_ms":started.elapsed().as_millis(),"message":error.to_string()}),
                    );
                    respond_error(
                        request,
                        &trace,
                        502,
                        "upstream_connection_error",
                        &format!("无法连接上游：{error}"),
                    );
                    return;
                }
            };
            log_upstream_response(&trace, &upstream, 1, started.elapsed());
            if !upstream.status().is_success() {
                respond_upstream_error(request, upstream, &trace);
                return;
            }
            if response_media_type(&upstream) != Some("text/event-stream") {
                let body = read_limited(&mut upstream, MAX_ERROR_BYTES).unwrap_or_default();
                log_upstream_error_body(&trace, &body);
                respond_error(
                    request,
                    &trace,
                    502,
                    "unsupported_upstream_response",
                    "上游成功响应必须使用 text/event-stream。",
                );
                return;
            }
            let upstream = ObservedReader::sse(upstream, trace.clone(), "upstream");
            let stream = openai_responses::ResponseStream::new(upstream, protocol, encoded);
            let stream = ObservedReader::sse(stream, trace.clone(), "downstream");
            respond_stream(request, stream, Vec::new(), &trace);
        }
    }
}

fn gateway_account(state: &AppState, token: &str) -> Result<GatewayAccount, String> {
    let connection = open_database(state)?;
    if !is_enabled(&connection)? {
        return Err("Codex 网关模式未启用。".to_string());
    }
    let local_key =
        local_api_key(&connection)?.ok_or_else(|| "本地 API Key 不存在。".to_string())?;
    if credential_fingerprint(token) != credential_fingerprint(&local_key) {
        return Err("本地 API Key 无效。".to_string());
    }
    let profile_id = get_setting(&connection, ACTIVE_PROFILE_SETTING)?
        .ok_or_else(|| "尚未选择 Codex 账户。".to_string())?;
    let row = connection
        .query_row(
            "SELECT alias, email, account_type, api_base_url, auth_json, upstream_protocol, upstream_auth_mode, anthropic_max_tokens
             FROM accounts WHERE id = ?1 AND product = 'codex'",
            params![profile_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "当前 Codex 账户不存在。".to_string())?;
    if row.2 == ACCOUNT_TYPE_OAUTH {
        return Ok(GatewayAccount::OAuth {
            profile_id,
            alias: row.0,
            email: row.1,
        });
    }
    if row.2 != ACCOUNT_TYPE_RELAY {
        return Err("不支持该 Codex 账户类型。".to_string());
    }
    Ok(GatewayAccount::Relay {
        alias: row.0,
        email: row.1,
        base_url: row.3.ok_or_else(|| "账户缺少 API 地址。".to_string())?,
        api_key: extract_api_key(&row.4)?.ok_or_else(|| "账户缺少 API Key。".to_string())?,
        protocol: UpstreamProtocol::parse(&row.5)?,
        auth_mode: UpstreamAuthMode::parse(&row.6)?,
        anthropic_max_tokens: row.7,
    })
}

fn apply_upstream_auth(
    request: RequestBuilder,
    auth_mode: UpstreamAuthMode,
    api_key: &str,
) -> RequestBuilder {
    match auth_mode {
        UpstreamAuthMode::Bearer => request.bearer_auth(api_key),
        UpstreamAuthMode::XApiKey => request.header("x-api-key", api_key),
    }
}

fn upstream_url(base_url: &str, endpoint: GatewayEndpoint) -> String {
    format!("{}/{}", base_url.trim_end_matches('/'), endpoint.suffix())
}

fn forwarded_request_headers(
    request: &Request,
    endpoint: GatewayEndpoint,
) -> Vec<(String, String)> {
    FORWARDED_REQUEST_HEADERS
        .iter()
        .filter(|name| endpoint.forwards_header(name))
        .filter_map(|name| header(request, name).map(|value| ((*name).to_string(), value)))
        .collect()
}

fn apply_forwarded_headers(
    mut request: RequestBuilder,
    headers: &[(String, String)],
    oauth: bool,
) -> RequestBuilder {
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if oauth
        && !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("Originator"))
    {
        request = request.header("Originator", "codex_cli_rs");
    }
    request
}

fn respond_native(
    request: Request,
    mut upstream: UpstreamResponse,
    endpoint: GatewayEndpoint,
    trace: &RequestTrace,
) {
    if !upstream.status().is_success() {
        respond_upstream_error(request, upstream, trace);
        return;
    }
    let media_type = response_media_type(&upstream).map(str::to_string);
    // Codex 按 SSE 解析响应体；部分成功上游会漏报或错报 Content-Type。
    if endpoint.requires_response_media_type() && media_type.as_deref() != Some(endpoint.accept()) {
        let body = read_limited(&mut upstream, MAX_ERROR_BYTES).unwrap_or_default();
        log_upstream_error_body(trace, &body);
        respond_error(
            request,
            trace,
            502,
            "unsupported_upstream_response",
            &format!(
                "上游成功响应必须使用 {}，实际为 {}。",
                endpoint.accept(),
                media_type.as_deref().unwrap_or("未声明 Content-Type")
            ),
        );
        return;
    }
    let headers = forwarded_response_headers(&upstream);
    let status = upstream.status().as_u16();
    match endpoint {
        GatewayEndpoint::Responses => {
            let upstream = ObservedReader::sse(upstream, trace.clone(), "upstream");
            let downstream = ObservedReader::sse(upstream, trace.clone(), "downstream");
            respond_stream(request, downstream, headers, trace);
        }
        GatewayEndpoint::Compact => {
            let upstream = ObservedReader::json(upstream, trace.clone(), "upstream");
            let downstream = ObservedReader::json(upstream, trace.clone(), "downstream");
            respond_json_stream(request, downstream, status, headers, trace);
        }
    }
}

fn respond_stream<R: Read + Send + 'static>(
    request: Request,
    stream: R,
    mut headers: Vec<Header>,
    trace: &RequestTrace,
) {
    headers.extend([
        header_value("Content-Type", "text/event-stream; charset=utf-8"),
        header_value("Cache-Control", "no-cache, no-store"),
        header_value("X-Content-Type-Options", "nosniff"),
        header_value("X-Cortana-Request-Id", trace.id()),
    ]);
    let response = Response::new(StatusCode(200), headers, stream, None, None);
    finish_response(trace, 200, request.respond(response));
}

fn respond_json_stream<R: Read + Send + 'static>(
    request: Request,
    stream: R,
    status: u16,
    mut headers: Vec<Header>,
    trace: &RequestTrace,
) {
    headers.extend([
        header_value("Content-Type", "application/json; charset=utf-8"),
        header_value("Cache-Control", "no-store"),
        header_value("X-Content-Type-Options", "nosniff"),
        header_value("X-Cortana-Request-Id", trace.id()),
    ]);
    let response = Response::new(StatusCode(status), headers, stream, None, None);
    finish_response(trace, status, request.respond(response));
}

fn forwarded_response_headers(upstream: &UpstreamResponse) -> Vec<Header> {
    upstream
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str();
            let allowed = is_forwarded_response_header(name);
            allowed
                .then(|| value.to_str().ok().map(|value| header_value(name, value)))
                .flatten()
        })
        .collect()
}

fn is_forwarded_response_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("x-request-id")
        || name.eq_ignore_ascii_case("request-id")
        || name.eq_ignore_ascii_case("x-openai-request-id")
        || name.eq_ignore_ascii_case("retry-after")
        || name.starts_with("x-codex-")
        || name.starts_with("x-ratelimit-")
        || name.starts_with("openai-")
}

fn response_media_type(response: &UpstreamResponse) -> Option<&str> {
    media_type(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
    )
}

fn log_upstream_response(
    trace: &RequestTrace,
    response: &UpstreamResponse,
    attempt: u32,
    elapsed: Duration,
) {
    let headers = response
        .headers()
        .iter()
        .filter(|(name, _)| is_forwarded_response_header(name.as_str()))
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.to_string(), json!(value)))
        })
        .collect::<serde_json::Map<_, _>>();
    trace.event(
        "info",
        "upstream.response",
        json!({
            "attempt":attempt,
            "status":response.status().as_u16(),
            "content_type":response_media_type(response),
            "response_header_ms":elapsed.as_millis(),
            "headers":headers,
        }),
    );
}

fn log_upstream_error_body(trace: &RequestTrace, body: &[u8]) {
    trace.payload_bytes("upstream.error.body", "upstream", body);
}

fn safe_upstream_url(value: &str) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return "[invalid-url]".to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

fn finish_response(trace: &RequestTrace, status: u16, result: std::io::Result<()>) {
    match result {
        Ok(()) if status < 400 => trace.event(
            "info",
            "request.completed",
            json!({"status":status,"duration_ms":trace.elapsed_ms()}),
        ),
        Ok(()) => trace.event(
            "info",
            "response.sent",
            json!({"status":status,"duration_ms":trace.elapsed_ms()}),
        ),
        Err(error) => trace.event(
            "error",
            "response.send_failed",
            json!({"status":status,"duration_ms":trace.elapsed_ms(),"message":error.to_string()}),
        ),
    }
}

fn respond_upstream_error(request: Request, mut upstream: UpstreamResponse, trace: &RequestTrace) {
    let status = upstream.status().as_u16();
    let headers = forwarded_response_headers(&upstream);
    let body = read_limited(&mut upstream, MAX_ERROR_BYTES).unwrap_or_default();
    log_upstream_error_body(trace, &body);
    let message = if body.len() <= MAX_ERROR_BYTES {
        backend_error_message(&String::from_utf8_lossy(&body))
    } else {
        format!("上游返回 HTTP {status}。")
    };
    respond_error_with_headers(request, trace, status, "upstream_error", &message, headers);
}

fn read_limited(reader: &mut impl Read, limit: usize) -> Result<Vec<u8>, std::io::Error> {
    let mut body = Vec::new();
    reader.take((limit + 1) as u64).read_to_end(&mut body)?;
    Ok(body)
}

fn bearer_token(value: Option<&str>) -> Option<&str> {
    value?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn media_type(value: Option<&str>) -> Option<&str> {
    value?.split(';').next().map(str::trim)
}

fn is_local_host(host: &str) -> bool {
    matches!(host, "127.0.0.1:11457" | "localhost:11457")
}

fn header(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.to_string().eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str().to_string())
}

fn header_value(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid HTTP header")
}

fn respond_error(request: Request, trace: &RequestTrace, status: u16, code: &str, message: &str) {
    respond_error_with_headers(request, trace, status, code, message, Vec::new());
}

fn respond_error_with_headers(
    request: Request,
    trace: &RequestTrace,
    status: u16,
    code: &str,
    message: &str,
    mut headers: Vec<Header>,
) {
    let body = serde_json::to_vec(&json!({
        "error": { "type": "invalid_request_error", "code": code, "message": message },
        "request_id": trace.id(),
    }))
    .expect("error JSON serialization cannot fail");
    headers.extend([
        header_value("Content-Type", "application/json; charset=utf-8"),
        header_value("Cache-Control", "no-store"),
        header_value("X-Cortana-Request-Id", trace.id()),
    ]);
    let response = Response::new(
        StatusCode(status),
        headers,
        body.as_slice(),
        Some(body.len()),
        None,
    );
    trace.event(
        if status >= 500 { "error" } else { "warn" },
        if status >= 500 {
            "request.failed"
        } else {
            "request.rejected"
        },
        json!({"status":status,"code":code,"message":message,"duration_ms":trace.elapsed_ms()}),
    );
    finish_response(trace, status, request.respond(response));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_fixed_gateway_path_and_bearer() {
        assert_eq!(
            GatewayEndpoint::parse("/v1/responses"),
            Some(GatewayEndpoint::Responses)
        );
        assert_eq!(
            GatewayEndpoint::parse("/v1/responses/compact"),
            Some(GatewayEndpoint::Compact)
        );
        assert_eq!(GatewayEndpoint::parse("/v1/chat/completions"), None);
        assert_eq!(bearer_token(Some("Bearer secret")), Some("secret"));
        assert_eq!(bearer_token(Some("Basic secret")), None);
    }

    #[test]
    fn builds_endpoint_specific_upstream_contract() {
        assert_eq!(
            upstream_url("https://example.com/v1/", GatewayEndpoint::Responses),
            "https://example.com/v1/responses"
        );
        assert_eq!(
            upstream_url("https://example.com/v1", GatewayEndpoint::Compact),
            "https://example.com/v1/responses/compact"
        );
        assert!(is_forwarded_response_header("retry-after"));
        assert!(is_forwarded_response_header("x-openai-request-id"));
        assert!(!is_forwarded_response_header("set-cookie"));
        for name in [
            "session-id",
            "thread-id",
            "x-client-request-id",
            "x-codex-turn-metadata",
            "x-openai-subagent",
        ] {
            assert!(FORWARDED_REQUEST_HEADERS.contains(&name));
        }
        assert!(!FORWARDED_REQUEST_HEADERS.contains(&"Session_id"));
        assert!(!FORWARDED_REQUEST_HEADERS.contains(&"Conversation_id"));
        assert!(GatewayEndpoint::Responses.forwards_header("x-client-request-id"));
        assert!(!GatewayEndpoint::Compact.forwards_header("x-client-request-id"));
        assert!(!GatewayEndpoint::Responses.requires_response_media_type());
        assert!(GatewayEndpoint::Compact.requires_response_media_type());
        assert_eq!(
            safe_upstream_url("https://user:secret@example.com/v1/responses?api_key=secret#x"),
            "https://example.com/v1/responses"
        );
    }

    #[test]
    fn normalizes_codex_oauth_requests() {
        let input = openai_responses::prepare_codex_oauth_request(
            json!({
                "model": "gpt-test",
                "input": "hello",
                "previous_response_id": "old"
            }),
            Some("session-1"),
            Some("install-1"),
        )
        .unwrap();
        assert_eq!(input["stream"], true);
        assert_eq!(input["store"], false);
        assert_eq!(input["input"][0]["content"], "hello");
        assert_eq!(input["prompt_cache_key"], "session-1");
        assert_eq!(
            input["client_metadata"]["x-codex-installation-id"],
            "install-1"
        );
        assert!(input.get("previous_response_id").is_none());
    }
}
