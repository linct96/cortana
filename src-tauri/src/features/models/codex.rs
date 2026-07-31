use super::types::{
    ModelEntry, CATALOG_FILE_NAME, DEFAULT_CATALOG_CONFIG_PATH, MODEL_CATALOG_TEMPLATE,
};
use crate::{
    platform::{
        db::{database_error, open_database},
        state::AppState,
    },
    products::codex::auth_path,
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::path::PathBuf;
use toml_edit::{value as toml_value, DocumentMut};

pub(crate) fn apply_model_config(
    state: &AppState,
    content: &str,
    profile_id: Option<&str>,
    default_model_id: Option<&str>,
) -> Result<(String, (PathBuf, Option<String>)), String> {
    let mut document = if content.trim().is_empty() {
        DocumentMut::new()
    } else {
        content
            .parse::<DocumentMut>()
            .map_err(|error| format!("config.toml 格式错误：{error}"))?
    };
    let codex_home = auth_path(state)?
        .parent()
        .ok_or_else(|| "Codex 主目录无效。".to_string())?
        .to_path_buf();
    let catalog_path = codex_home.join(CATALOG_FILE_NAME);
    let Some(profile_id) = profile_id else {
        clear_model_config(&mut document);
        return Ok((document.to_string(), (catalog_path, None)));
    };
    let template = model_catalog_template()?;
    let connection = open_database(state)?;
    let models_json = connection
        .query_row(
            "SELECT models_json FROM model_profiles WHERE id = ?1 AND product = 'codex'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "关联的模型方案不存在。".to_string())?;
    let models: Vec<ModelEntry> =
        serde_json::from_str(&models_json).map_err(|_| "模型方案数据已损坏。".to_string())?;
    if models.is_empty() {
        return Err("关联的模型方案至少需要一个模型。".to_string());
    }
    let default_model_id = default_model_id.ok_or_else(|| "关联账号缺少默认模型。".to_string())?;
    if !models.iter().any(|model| model.id == default_model_id) {
        return Err("账号默认模型不在关联方案中。".to_string());
    }
    let catalog = generate_catalog(&template, &models, default_model_id)?;
    let catalog_config_path = if codex_home == state.default_codex_home {
        DEFAULT_CATALOG_CONFIG_PATH.to_string()
    } else {
        catalog_path.display().to_string()
    };
    document["model"] = toml_value(default_model_id);
    document["model_catalog_json"] = toml_value(catalog_config_path);
    Ok((document.to_string(), (catalog_path, Some(catalog))))
}

pub(super) fn clear_model_config(document: &mut DocumentMut) {
    remove_config_key(document, "model_catalog_json");
    remove_config_key(document, "model");
}

pub(super) fn remove_config_key(document: &mut DocumentMut, key: &str) {
    let prefix = document
        .as_table()
        .key(key)
        .and_then(|key| key.leaf_decor().prefix().and_then(|raw| raw.as_str()))
        .unwrap_or_default()
        .to_string();
    document.as_table_mut().remove(key);
    if !prefix.trim().is_empty() {
        let trailing = document.trailing().as_str().unwrap_or_default();
        document.set_trailing(format!("{trailing}{prefix}"));
    }
}

pub(super) fn model_catalog_template() -> Result<Value, String> {
    serde_json::from_str(MODEL_CATALOG_TEMPLATE)
        .map_err(|_| "项目内置 Codex 模型模板格式无效。".to_string())
}

pub(super) fn generate_catalog(
    bundled: &Value,
    models: &[ModelEntry],
    default_model_id: &str,
) -> Result<String, String> {
    let bundled_models = bundled
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| "项目内置 Codex 模型模板缺少 models。".to_string())?;
    let template = bundled_models
        .iter()
        .filter(|model| {
            model.get("supported_in_api").and_then(Value::as_bool) == Some(true)
                && model.get("use_responses_lite").and_then(Value::as_bool) != Some(true)
                && model.get("multi_agent_version").is_none_or(Value::is_null)
                && model.get("tool_mode").is_none_or(Value::is_null)
        })
        .min_by_key(|model| {
            model
                .get("priority")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX)
        })
        .ok_or_else(|| "项目内置 Codex 模型模板缺少可用模型。".to_string())?;
    let mut ordered = models.to_vec();
    ordered.sort_by(|left, right| {
        (left.id != default_model_id)
            .cmp(&(right.id != default_model_id))
            .then_with(|| {
                left.display_name
                    .to_lowercase()
                    .cmp(&right.display_name.to_lowercase())
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    let generated = ordered
        .into_iter()
        .enumerate()
        .map(|(priority, model)| {
            let mut value = template.clone();
            let object = value.as_object_mut().expect("bundled model must be object");
            object.insert("slug".to_string(), Value::String(model.id));
            object.insert(
                "description".to_string(),
                Value::String(model.display_name.clone()),
            );
            object.insert(
                "display_name".to_string(),
                Value::String(model.display_name),
            );
            object.insert("priority".to_string(), json!(priority + 1));
            if model.context_1m {
                object.insert("context_window".to_string(), json!(1_048_576));
                object.insert("max_context_window".to_string(), json!(1_048_576));
            }
            value
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&json!({ "models": generated })).map_err(|error| error.to_string())
}
