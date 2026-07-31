use super::{remote::normalize_models, types::ClaudeModelSlot};
use crate::platform::{
    db::{database_error, open_database},
    state::{AccountProduct, AppState},
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

pub(crate) fn apply_claude_model_config(
    state: &AppState,
    settings: &mut Value,
    profile_id: Option<&str>,
    default_model_id: Option<&str>,
) -> Result<(), String> {
    let had_model_mapping =
        settings
            .get("env")
            .and_then(Value::as_object)
            .is_some_and(|environment| {
                [
                    ClaudeModelSlot::Fable,
                    ClaudeModelSlot::Opus,
                    ClaudeModelSlot::Sonnet,
                    ClaudeModelSlot::Haiku,
                    ClaudeModelSlot::Custom,
                ]
                .into_iter()
                .any(|slot| environment.contains_key(claude_slot_keys(slot).0))
            });
    if profile_id.is_none() && !had_model_mapping {
        return Ok(());
    }
    clear_claude_model_config(settings)?;
    let Some(profile_id) = profile_id else {
        if had_model_mapping {
            let root = settings
                .as_object_mut()
                .ok_or_else(|| "Claude settings.json 必须是一个 JSON 对象。".to_string())?;
            root.remove("model");
        }
        return Ok(());
    };

    let connection = open_database(state)?;
    let models_json = connection
        .query_row(
            "SELECT models_json FROM model_profiles WHERE id = ?1 AND product = 'claude'",
            params![profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "关联的 Claude 模型方案不存在。".to_string())?;
    let models = normalize_models(
        AccountProduct::Claude,
        serde_json::from_str(&models_json).map_err(|_| "模型方案数据已损坏。".to_string())?,
    )?;
    let default_model_id = default_model_id.ok_or_else(|| "关联账号缺少默认模型。".to_string())?;
    let default_model = models
        .iter()
        .find(|model| model.id == default_model_id)
        .ok_or_else(|| "账号默认模型不在关联方案中。".to_string())?;
    let root = settings
        .as_object_mut()
        .ok_or_else(|| "Claude settings.json 必须是一个 JSON 对象。".to_string())?;
    let environment = root
        .entry("env".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "Claude settings.json 的 env 必须是一个 JSON 对象。".to_string())?;

    for slot in [
        ClaudeModelSlot::Fable,
        ClaudeModelSlot::Opus,
        ClaudeModelSlot::Sonnet,
        ClaudeModelSlot::Haiku,
    ] {
        let (key, name_key, _) = claude_slot_keys(slot);
        let Some(model) = models.iter().find(|model| model.claude_slot == Some(slot)) else {
            continue;
        };
        let model_id = if model.context_1m {
            format!("{}[1m]", model.id.strip_suffix("[1m]").unwrap_or(&model.id))
        } else {
            model.id.clone()
        };
        environment.insert(key.to_string(), Value::String(model_id));
        environment.insert(
            name_key.to_string(),
            Value::String(model.display_name.clone()),
        );
    }
    if let Some(model) = models
        .iter()
        .find(|model| model.claude_slot == Some(ClaudeModelSlot::Custom))
    {
        let (key, name_key, _) = claude_slot_keys(ClaudeModelSlot::Custom);
        environment.insert(key.to_string(), Value::String(model.id.clone()));
        environment.insert(
            name_key.to_string(),
            Value::String(model.display_name.clone()),
        );
    }
    let selector = if default_model.claude_slot == Some(ClaudeModelSlot::Custom) {
        default_model.id.clone()
    } else {
        claude_slot_keys(
            default_model
                .claude_slot
                .ok_or_else(|| "Claude 模型缺少映射入口。".to_string())?,
        )
        .2
        .to_string()
    };
    root.insert("model".to_string(), Value::String(selector));
    Ok(())
}

pub(super) fn clear_claude_model_config(settings: &mut Value) -> Result<(), String> {
    let root = settings
        .as_object_mut()
        .ok_or_else(|| "Claude settings.json 必须是一个 JSON 对象。".to_string())?;
    if let Some(environment) = root.get_mut("env").and_then(Value::as_object_mut) {
        for slot in [
            ClaudeModelSlot::Fable,
            ClaudeModelSlot::Opus,
            ClaudeModelSlot::Sonnet,
            ClaudeModelSlot::Haiku,
            ClaudeModelSlot::Custom,
        ] {
            let (key, name_key, _) = claude_slot_keys(slot);
            environment.remove(key);
            environment.remove(name_key);
            environment.remove(&format!("{key}_DESCRIPTION"));
            environment.remove(&format!("{key}_SUPPORTED_CAPABILITIES"));
        }
    }
    Ok(())
}

pub(super) fn claude_slot_keys(
    slot: ClaudeModelSlot,
) -> (&'static str, &'static str, &'static str) {
    match slot {
        ClaudeModelSlot::Fable => (
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
            "fable",
        ),
        ClaudeModelSlot::Opus => (
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
            "opus",
        ),
        ClaudeModelSlot::Sonnet => (
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
            "sonnet",
        ),
        ClaudeModelSlot::Haiku => (
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            "haiku",
        ),
        ClaudeModelSlot::Custom => (
            "ANTHROPIC_CUSTOM_MODEL_OPTION",
            "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
            "",
        ),
    }
}
