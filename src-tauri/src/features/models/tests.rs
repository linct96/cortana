use super::{
    claude::apply_claude_model_config,
    codex::{generate_catalog, model_catalog_template},
    commands::delete_model_profile_internal,
    grok::{apply_grok_model_config, grok_model_key},
    remote::{normalize_models, parse_remote_models},
    store::save_model_profile,
    types::{ClaudeModelSlot, ModelAssignment, ModelEntry},
};
use crate::{
    features::accounts,
    platform::{
        db::{initialize_database, open_database},
        state::{AccountProduct, AppState},
    },
    products::grok as product_grok,
};
use serde_json::{json, Value};
use std::{
    fs,
    sync::{Arc, Mutex},
};
use toml_edit::{value as toml_value, DocumentMut};
use uuid::Uuid;

#[test]
fn parses_remote_model_shapes_and_deduplicates() {
    let models = parse_remote_models(&json!({
        "data": [
            { "id": "deepseek-chat", "name": "DeepSeek Chat" },
            { "id": "deepseek-chat" },
            { "id": "deepseek-reasoner", "display_name": "DeepSeek Reasoner" }
        ]
    }))
    .unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].display_name, "DeepSeek Chat");
}

#[test]
fn rejects_empty_model_profile() {
    assert_eq!(
        normalize_models(AccountProduct::Codex, Vec::new()).unwrap_err(),
        "模型方案至少需要一个模型。"
    );
}

#[test]
fn preserves_codex_1m_context() {
    let models = normalize_models(
        AccountProduct::Codex,
        vec![ModelEntry {
            id: "custom".to_string(),
            display_name: "Custom".to_string(),
            claude_slot: None,
            context_1m: true,
        }],
    )
    .unwrap();
    assert!(models[0].context_1m);
}

#[test]
fn allows_duplicate_model_ids() {
    let models = normalize_models(
        AccountProduct::Codex,
        vec![
            ModelEntry {
                id: "same".to_string(),
                display_name: "First".to_string(),
                claude_slot: None,
                context_1m: false,
            },
            ModelEntry {
                id: "same".to_string(),
                display_name: "Second".to_string(),
                claude_slot: None,
                context_1m: false,
            },
        ],
    )
    .unwrap();
    assert_eq!(models.len(), 2);
}

#[test]
fn generates_default_first_catalog_from_generic_template() {
    let bundled = json!({
        "models": [{
            "slug": "gpt-template",
            "display_name": "Template",
            "supported_in_api": true,
            "priority": 7,
            "use_responses_lite": false,
            "multi_agent_version": null,
            "tool_mode": null,
            "base_instructions": "base",
            "service_tiers": ["template-tier"]
        }]
    });
    let content = generate_catalog(
        &bundled,
        &[
            ModelEntry {
                id: "z-model".to_string(),
                display_name: "Z Model".to_string(),
                claude_slot: None,
                context_1m: false,
            },
            ModelEntry {
                id: "a-model".to_string(),
                display_name: "A Model".to_string(),
                claude_slot: None,
                context_1m: false,
            },
        ],
        "z-model",
    )
    .unwrap();
    let catalog: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(catalog["models"][0]["slug"], "z-model");
    assert_eq!(catalog["models"][0]["priority"], 1);
    assert_eq!(catalog["models"][1]["slug"], "a-model");
    assert_eq!(catalog["models"][0]["base_instructions"], "base");
    assert_eq!(catalog["models"][0]["description"], "Z Model");
    assert_eq!(
        catalog["models"][0]["service_tiers"],
        json!(["template-tier"])
    );
}

#[test]
fn loads_embedded_model_catalog_template() {
    let template = model_catalog_template().unwrap();
    assert!(template["models"][0]["base_instructions"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
}

#[test]
fn aggregates_grok_accounts_with_independent_credentials() {
    let directory =
        std::env::temp_dir().join(format!("cortana-grok-model-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let state = AppState {
        database_path: directory.join("app.sqlite3"),
        default_codex_home: directory.clone(),
        pending_oauth: Arc::new(Mutex::new(None)),
    };
    initialize_database(&state).unwrap();
    let first = product_grok::upsert_relay_profile(
        &state,
        "first-key",
        "https://first.example/v1",
        "First",
    )
    .unwrap();
    let second = product_grok::upsert_relay_profile(
        &state,
        "second-key",
        "https://second.example/v1",
        "Second",
    )
    .unwrap();
    save_model_profile(
        &state,
        AccountProduct::Grok,
        None,
        "Grok",
        vec![
            ModelEntry {
                id: "chat".to_string(),
                display_name: "Chat".to_string(),
                claude_slot: None,
                context_1m: false,
            },
            ModelEntry {
                id: "reasoner".to_string(),
                display_name: "Reasoner".to_string(),
                claude_slot: None,
                context_1m: false,
            },
        ],
        vec![
            ModelAssignment {
                account_id: first.id.clone(),
                account_alias: first.alias.clone(),
                default_model_id: Some("chat".to_string()),
            },
            ModelAssignment {
                account_id: second.id.clone(),
                account_alias: second.alias.clone(),
                default_model_id: Some("reasoner".to_string()),
            },
        ],
        false,
    )
    .unwrap();
    let mut document = "[ui]\nyolo = false\n".parse::<DocumentMut>().unwrap();
    let connection = open_database(&state).unwrap();

    apply_grok_model_config(
        &connection,
        &mut document,
        &[first.id.clone(), second.id.clone()],
    )
    .unwrap();
    let first_key = grok_model_key(&first.id, 0);
    let second_key = grok_model_key(&second.id, 1);
    assert_eq!(
        document["models"]["default"].as_str(),
        Some(first_key.as_str())
    );
    assert_eq!(
        document["model"][&first_key]["api_key"].as_str(),
        Some("first-key")
    );
    assert_eq!(
        document["model"][&first_key]["description"].as_str(),
        Some("由 First 提供")
    );
    assert_eq!(
        document["model"][&second_key]["api_key"].as_str(),
        Some("second-key")
    );
    document["models"]["default"] = toml_value(&second_key);
    apply_grok_model_config(
        &connection,
        &mut document,
        &[first.id.clone(), second.id.clone()],
    )
    .unwrap();
    assert_eq!(
        document["models"]["default"].as_str(),
        Some(second_key.as_str())
    );

    let colliding_id = format!("{}-different-account", &first.id[..8]);
    let error = apply_grok_model_config(
        &connection,
        &mut document,
        &[first.id.clone(), colliding_id],
    )
    .unwrap_err();
    assert!(error.contains("短 ID 冲突"), "{error}");

    apply_grok_model_config(&connection, &mut document, &[]).unwrap();
    assert!(document["model"].get(&first_key).is_none());
    assert!(document["model"].get(&second_key).is_none());
    assert!(document["models"].get("default").is_none());
    assert_eq!(document["ui"]["yolo"].as_bool(), Some(false));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn applies_and_clears_claude_model_mapping() {
    let directory =
        std::env::temp_dir().join(format!("cortana-claude-model-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let state = AppState {
        database_path: directory.join("app.sqlite3"),
        default_codex_home: directory.clone(),
        pending_oauth: Arc::new(Mutex::new(None)),
    };
    initialize_database(&state).unwrap();
    let profile = save_model_profile(
        &state,
        AccountProduct::Claude,
        None,
        "Claude",
        vec![
            ModelEntry {
                id: "step-fable".to_string(),
                display_name: "Step Fable".to_string(),
                claude_slot: Some(ClaudeModelSlot::Fable),
                context_1m: true,
            },
            ModelEntry {
                id: "step-opus".to_string(),
                display_name: "Step Opus".to_string(),
                claude_slot: Some(ClaudeModelSlot::Opus),
                context_1m: false,
            },
            ModelEntry {
                id: "step-custom".to_string(),
                display_name: "Step Custom".to_string(),
                claude_slot: Some(ClaudeModelSlot::Custom),
                context_1m: false,
            },
        ],
        Vec::new(),
        false,
    )
    .unwrap();
    let mut settings = json!({
        "env": {
            "ANTHROPIC_BASE_URL": "https://relay.example.com",
            "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY": "1"
        },
        "availableModels": ["user-model"],
        "includeCoAuthoredBy": false
    });

    apply_claude_model_config(&state, &mut settings, Some(&profile.id), Some("step-opus")).unwrap();

    assert_eq!(settings["model"], "opus");
    assert_eq!(settings["availableModels"], json!(["user-model"]));
    assert_eq!(
        settings["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"],
        "step-fable[1m]"
    );
    assert_eq!(settings["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "step-opus");
    assert!(settings["env"]
        .get("ANTHROPIC_DEFAULT_SONNET_MODEL")
        .is_none());
    assert!(settings["env"]
        .get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
        .is_none());
    assert_eq!(
        settings["env"]["ANTHROPIC_CUSTOM_MODEL_OPTION"],
        "step-custom"
    );
    assert_eq!(
        settings["env"]["CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY"],
        "1"
    );
    apply_claude_model_config(&state, &mut settings, None, None).unwrap();
    assert!(settings.get("model").is_none());
    assert_eq!(settings["availableModels"], json!(["user-model"]));
    assert!(settings["env"]
        .get("ANTHROPIC_DEFAULT_FABLE_MODEL")
        .is_none());
    assert!(settings["env"]
        .get("ANTHROPIC_DEFAULT_OPUS_MODEL")
        .is_none());
    assert_eq!(settings["includeCoAuthoredBy"], false);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn rejects_duplicate_claude_slots() {
    let error = normalize_models(
        AccountProduct::Claude,
        vec![
            ModelEntry {
                id: "first".to_string(),
                display_name: "First".to_string(),
                claude_slot: Some(ClaudeModelSlot::Opus),
                context_1m: false,
            },
            ModelEntry {
                id: "second".to_string(),
                display_name: "Second".to_string(),
                claude_slot: Some(ClaudeModelSlot::Opus),
                context_1m: false,
            },
        ],
    )
    .unwrap_err();
    assert_eq!(error, "Claude 模型映射入口重复。");
}

#[test]
fn rejects_1m_custom_model() {
    let error = normalize_models(
        AccountProduct::Claude,
        vec![ModelEntry {
            id: "custom".to_string(),
            display_name: "Custom".to_string(),
            claude_slot: Some(ClaudeModelSlot::Custom),
            context_1m: true,
        }],
    )
    .unwrap_err();
    assert_eq!(error, "Custom 模型不支持 1M 上下文配置。");
}

#[test]
fn saves_shared_profile_with_per_account_defaults() {
    let directory = std::env::temp_dir().join(format!("cortana-model-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let state = AppState {
        database_path: directory.join("app.sqlite3"),
        default_codex_home: directory.clone(),
        pending_oauth: Arc::new(Mutex::new(None)),
    };
    initialize_database(&state).unwrap();
    let first = accounts::upsert_relay_profile(
        &state,
        "first-key",
        "https://first.example.com/v1",
        "First",
    )
    .unwrap();
    let second = accounts::upsert_relay_profile(
        &state,
        "second-key",
        "https://second.example.com/v1",
        "Second",
    )
    .unwrap();
    let profile = save_model_profile(
        &state,
        AccountProduct::Codex,
        None,
        "Shared",
        vec![
            ModelEntry {
                id: "chat".to_string(),
                display_name: "Chat".to_string(),
                claude_slot: None,
                context_1m: false,
            },
            ModelEntry {
                id: "reasoner".to_string(),
                display_name: "Reasoner".to_string(),
                claude_slot: None,
                context_1m: false,
            },
        ],
        vec![
            ModelAssignment {
                account_id: first.id,
                account_alias: "First".to_string(),
                default_model_id: Some("chat".to_string()),
            },
            ModelAssignment {
                account_id: second.id,
                account_alias: "Second".to_string(),
                default_model_id: Some("reasoner".to_string()),
            },
        ],
        false,
    )
    .unwrap();
    assert_eq!(profile.assignments.len(), 2);
    assert!(delete_model_profile_internal(&state, AccountProduct::Codex, &profile.id).is_err());
    fs::remove_dir_all(directory).unwrap();
}
