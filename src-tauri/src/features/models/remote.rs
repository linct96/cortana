use super::types::{ClaudeModelSlot, ModelEntry, RelayModelOption, MAX_MODELS_RESPONSE_BYTES};
use crate::{
    features::{accounts, gateway::UpstreamAuthMode},
    platform::{
        db::{database_error, open_database},
        state::{AccountProduct, AppState},
    },
    products::codex::normalize_api_base_url,
};
use reqwest::blocking::Client;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    time::Duration,
};

pub(super) fn normalize_models(
    product: AccountProduct,
    models: Vec<ModelEntry>,
) -> Result<Vec<ModelEntry>, String> {
    if models.is_empty() {
        return Err("模型方案至少需要一个模型。".to_string());
    }
    let mut claude_slots = HashSet::new();
    let mut result = Vec::with_capacity(models.len());
    for model in models {
        let id = model.id.trim();
        let display_name = model.display_name.trim();
        if id.is_empty() || display_name.is_empty() {
            return Err("模型 ID 和显示名称不能为空。".to_string());
        }
        let claude_slot = match product {
            AccountProduct::Claude => {
                let slot = model
                    .claude_slot
                    .ok_or_else(|| "Claude 模型必须选择映射入口。".to_string())?;
                if slot == ClaudeModelSlot::Custom && model.context_1m {
                    return Err("Custom 模型不支持 1M 上下文配置。".to_string());
                }
                if !claude_slots.insert(slot) {
                    return Err("Claude 模型映射入口重复。".to_string());
                }
                Some(slot)
            }
            AccountProduct::Codex | AccountProduct::Grok => None,
            AccountProduct::Antigravity => return Err("该产品暂不支持自定义模型。".to_string()),
        };
        result.push(ModelEntry {
            id: id.to_string(),
            display_name: display_name.to_string(),
            claude_slot,
            context_1m: matches!(product, AccountProduct::Codex | AccountProduct::Claude)
                && model.context_1m,
        });
    }
    if product == AccountProduct::Claude && result.len() > 5 {
        return Err("Claude 模型方案最多支持 5 个模型。".to_string());
    }
    result.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(result)
}

pub(super) fn fetch_relay_models_internal(
    state: &AppState,
    product: AccountProduct,
    account_id: &str,
) -> Result<Vec<RelayModelOption>, String> {
    if !matches!(
        product,
        AccountProduct::Codex | AccountProduct::Claude | AccountProduct::Grok
    ) {
        return Err("该产品暂不支持中转站账户。".to_string());
    }
    let connection = open_database(state)?;
    let (api_base_url, upstream_auth_mode) = connection
        .query_row(
            "SELECT api_base_url, upstream_auth_mode FROM accounts WHERE id = ?1 AND product = ?2 AND account_type = 'relay'",
            params![account_id, product.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "中转账号不存在。".to_string())?;
    let api_key = accounts::relay_api_key_for_profile(&connection, account_id, product)?;
    let base_url = normalize_api_base_url(&api_base_url)?;
    let url = match product {
        AccountProduct::Codex | AccountProduct::Grok => format!("{base_url}/models"),
        AccountProduct::Claude => format!("{base_url}/v1/models"),
        AccountProduct::Antigravity => unreachable!(),
    };
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;
    let request = client.get(url);
    let request = match UpstreamAuthMode::parse(&upstream_auth_mode)? {
        UpstreamAuthMode::Bearer => request.bearer_auth(api_key),
        UpstreamAuthMode::XApiKey => request.header("x-api-key", api_key),
    };
    let mut response = request
        .send()
        .map_err(|error| format!("同步远端模型失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("同步远端模型失败：HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_MODELS_RESPONSE_BYTES as u64)
    {
        return Err("远端模型响应过大。".to_string());
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take((MAX_MODELS_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("无法读取远端模型：{error}"))?;
    if bytes.len() > MAX_MODELS_RESPONSE_BYTES {
        return Err("远端模型响应过大。".to_string());
    }
    parse_remote_models(
        &serde_json::from_slice(&bytes).map_err(|_| "远端模型响应不是有效的 JSON。".to_string())?,
    )
}

pub(super) fn parse_remote_models(value: &Value) -> Result<Vec<RelayModelOption>, String> {
    let items = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .ok_or_else(|| "远端模型响应缺少 data 或 models 数组。".to_string())?;
    let mut models = HashMap::new();
    for item in items {
        let Some(id) = item
            .get("id")
            .or_else(|| item.get("slug"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let display_name = item
            .get("display_name")
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(id);
        models
            .entry(id.to_string())
            .or_insert_with(|| RelayModelOption {
                id: id.to_string(),
                display_name: display_name.to_string(),
            });
    }
    let mut models = models.into_values().collect::<Vec<_>>();
    models.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(models)
}
