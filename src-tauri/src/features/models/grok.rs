use super::{remote::normalize_models, types::ModelEntry};
use crate::{
    platform::{
        db::{credential_fingerprint, database_error},
        state::AccountProduct,
    },
    products::codex::normalize_api_base_url,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::HashMap;
use toml_edit::{table as toml_table, value as toml_value, DocumentMut};

#[derive(Debug, Clone)]
pub(super) struct GrokRelayModels {
    account_id: String,
    alias: String,
    api_base_url: String,
    api_key: String,
    models: Vec<ModelEntry>,
    default_model_id: String,
}

pub(crate) fn apply_grok_model_config(
    connection: &Connection,
    document: &mut DocumentMut,
    enabled_account_ids: &[String],
) -> Result<(), String> {
    let accounts = load_grok_relay_models(connection, enabled_account_ids)?;
    let previous_default = current_grok_default(document, &accounts);
    clear_grok_model_config(document);
    if accounts.is_empty() {
        return Ok(());
    }

    if document.get("model").is_none() {
        document["model"] = toml_table();
    }
    for account in &accounts {
        for (index, model) in account.models.iter().enumerate() {
            let key = grok_model_key(&account.account_id, index);
            document["model"][&key] = toml_table();
            document["model"][&key]["model"] = toml_value(&model.id);
            document["model"][&key]["name"] = toml_value(&model.display_name);
            document["model"][&key]["description"] =
                toml_value(format!("由 {} 提供", account.alias));
            document["model"][&key]["base_url"] = toml_value(&account.api_base_url);
            document["model"][&key]["api_key"] = toml_value(&account.api_key);
            document["model"][&key]["api_backend"] = toml_value("chat_completions");
        }
    }
    if document.get("models").is_none() {
        document["models"] = toml_table();
    }
    let default = previous_default
        .and_then(|(account_id, model_id)| {
            accounts
                .iter()
                .find(|account| account.account_id == account_id)
                .and_then(|account| {
                    account
                        .models
                        .iter()
                        .position(|model| model.id == model_id)
                        .map(|index| grok_model_key(&account.account_id, index))
                })
        })
        .unwrap_or_else(|| {
            let account = &accounts[0];
            let index = account
                .models
                .iter()
                .position(|model| model.id == account.default_model_id)
                .expect("validated Grok default model");
            grok_model_key(&account.account_id, index)
        });
    document["models"]["default"] = toml_value(default);
    Ok(())
}

pub(crate) fn infer_grok_enabled_accounts(
    connection: &Connection,
    document: &DocumentMut,
) -> Result<Option<Vec<String>>, String> {
    let Some(table) = document.get("model").and_then(|item| item.as_table()) else {
        return Ok(None);
    };
    let nodes = table
        .iter()
        .filter(|(key, _)| key.starts_with("cortana-"))
        .map(|(_, item)| {
            let model_id = item
                .get("model")
                .and_then(|item| item.as_str())
                .ok_or_else(|| "Grok 托管模型缺少模型 ID。".to_string())?;
            let api_base_url = item
                .get("base_url")
                .and_then(|item| item.as_str())
                .ok_or_else(|| "Grok 托管模型缺少 API 地址。".to_string())?;
            let api_key = item
                .get("api_key")
                .and_then(|item| item.as_str())
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "Grok 托管模型缺少 API Key。".to_string())?;
            Ok((
                model_id.to_string(),
                normalize_api_base_url(api_base_url)?,
                credential_fingerprint(api_key),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if nodes.is_empty() {
        return Ok(None);
    }

    let mut statement = connection
        .prepare(
            "SELECT id, api_base_url, account_id FROM accounts WHERE product = 'grok' AND account_type = 'relay' ORDER BY sort_order, created_at",
        )
        .map_err(database_error)?;
    let candidates = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    let mut enabled = Vec::new();
    let mut configured_models: HashMap<String, Vec<String>> = HashMap::new();
    for (model_id, api_base_url, fingerprint) in nodes {
        let matches = candidates
            .iter()
            .filter(|(_, candidate_url, candidate_fingerprint)| {
                normalize_api_base_url(candidate_url).as_deref() == Ok(api_base_url.as_str())
                    && candidate_fingerprint == &fingerprint
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err("Grok 托管模型无法唯一匹配本地中转账号。".to_string());
        }
        let account_id = &matches[0].0;
        if !enabled.contains(account_id) {
            enabled.push(account_id.clone());
        }
        configured_models
            .entry(account_id.clone())
            .or_default()
            .push(model_id);
    }
    let accounts = load_grok_relay_models(connection, &enabled)?;
    for account in accounts {
        let mut expected = account
            .models
            .into_iter()
            .map(|model| model.id)
            .collect::<Vec<_>>();
        let mut configured = configured_models
            .remove(&account.account_id)
            .unwrap_or_default();
        expected.sort();
        configured.sort();
        if expected != configured {
            return Err(format!(
                "Grok 中转账号“{}”的托管模型与关联方案不一致。",
                account.alias
            ));
        }
    }
    Ok(Some(enabled))
}

pub(crate) fn grok_config_matches_accounts(
    connection: &Connection,
    document: &DocumentMut,
    enabled_account_ids: &[String],
) -> bool {
    if enabled_account_ids.is_empty() {
        return document
            .get("model")
            .and_then(|item| item.as_table())
            .is_none_or(|table| !table.iter().any(|(key, _)| key.starts_with("cortana-")));
    }
    let api_key_mode = document
        .get("auth")
        .and_then(|item| item.get("preferred_method"))
        .and_then(|item| item.as_str())
        == Some("api_key");
    let managed_default = document
        .get("models")
        .and_then(|item| item.get("default"))
        .and_then(|item| item.as_str())
        .is_some_and(|key| key.starts_with("cortana-"));
    api_key_mode
        && managed_default
        && infer_grok_enabled_accounts(connection, document)
            .ok()
            .flatten()
            .is_some_and(|configured| configured == enabled_account_ids)
}

pub(super) fn load_grok_relay_models(
    connection: &Connection,
    enabled_account_ids: &[String],
) -> Result<Vec<GrokRelayModels>, String> {
    let mut short_ids = HashMap::new();
    let mut result = Vec::with_capacity(enabled_account_ids.len());
    for account_id in enabled_account_ids {
        let short_id = account_id.chars().take(8).collect::<String>();
        if short_id.len() != 8 {
            return Err("Grok 中转账号 ID 无效。".to_string());
        }
        if short_ids
            .insert(short_id.clone(), account_id.as_str())
            .is_some()
        {
            return Err(format!("Grok 中转账号短 ID 冲突：{short_id}。"));
        }
        let (alias, api_base_url, auth_json, model_profile_id, default_model_id) = connection
            .query_row(
                "SELECT alias, api_base_url, auth_json, model_profile_id, default_model_id FROM accounts WHERE id = ?1 AND product = 'grok' AND account_type = 'relay'",
                params![account_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| "已启用的 Grok 中转账号不存在。".to_string())?;
        let api_key = serde_json::from_str::<Value>(&auth_json)
            .ok()
            .and_then(|value| value.get("key").and_then(Value::as_str).map(str::to_string))
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("Grok 中转账号“{alias}”缺少 API Key。"))?;
        let model_profile_id =
            model_profile_id.ok_or_else(|| format!("Grok 中转账号“{alias}”请先关联模型方案。"))?;
        let models_json = connection
            .query_row(
                "SELECT models_json FROM model_profiles WHERE id = ?1 AND product = 'grok'",
                params![model_profile_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| format!("Grok 中转账号“{alias}”关联的模型方案不存在。"))?;
        let models = normalize_models(
            AccountProduct::Grok,
            serde_json::from_str(&models_json).map_err(|_| "模型方案数据已损坏。".to_string())?,
        )?;
        let default_model_id =
            default_model_id.ok_or_else(|| format!("Grok 中转账号“{alias}”缺少默认模型。"))?;
        if !models.iter().any(|model| model.id == default_model_id) {
            return Err(format!("Grok 中转账号“{alias}”的默认模型不在关联方案中。"));
        }
        result.push(GrokRelayModels {
            account_id: account_id.clone(),
            alias,
            api_base_url: normalize_api_base_url(&api_base_url)?,
            api_key,
            models,
            default_model_id,
        });
    }
    Ok(result)
}

pub(super) fn current_grok_default(
    document: &DocumentMut,
    accounts: &[GrokRelayModels],
) -> Option<(String, String)> {
    let key = document
        .get("models")?
        .get("default")?
        .as_str()
        .filter(|key| key.starts_with("cortana-"))?;
    let model = document.get("model")?.get(key)?;
    let model_id = model.get("model")?.as_str()?;
    let api_base_url = normalize_api_base_url(model.get("base_url")?.as_str()?).ok()?;
    let fingerprint = credential_fingerprint(model.get("api_key")?.as_str()?);
    accounts
        .iter()
        .find(|account| {
            account.api_base_url == api_base_url
                && credential_fingerprint(&account.api_key) == fingerprint
                && account.models.iter().any(|model| model.id == model_id)
        })
        .map(|account| (account.account_id.clone(), model_id.to_string()))
}

pub(super) fn clear_grok_model_config(document: &mut DocumentMut) {
    let keys = document
        .get("model")
        .and_then(|item| item.as_table())
        .map(|table| {
            table
                .iter()
                .filter(|(key, _)| key.starts_with("cortana-"))
                .map(|(key, _)| key.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(table) = document
        .get_mut("model")
        .and_then(|item| item.as_table_mut())
    {
        for key in keys {
            table.remove(&key);
        }
    }
    if document
        .get("models")
        .and_then(|item| item.get("default"))
        .and_then(|item| item.as_str())
        .is_some_and(|value| value.starts_with("cortana-"))
    {
        document["models"]
            .as_table_mut()
            .map(|table| table.remove("default"));
    }
}

pub(super) fn grok_model_key(profile_id: &str, index: usize) -> String {
    format!(
        "cortana-{}-{index}",
        profile_id.chars().take(8).collect::<String>()
    )
}
