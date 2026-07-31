use serde::{Deserialize, Serialize};

pub(super) const CATALOG_FILE_NAME: &str = "cortana_models.json";
pub(super) const DEFAULT_CATALOG_CONFIG_PATH: &str = "~/.codex/cortana_models.json";
pub(super) const MODEL_CATALOG_TEMPLATE: &str =
    include_str!("../../../resources/codex-model-template.json");
pub(super) const MAX_MODELS_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ClaudeModelSlot {
    Fable,
    Opus,
    Sonnet,
    Haiku,
    Custom,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelEntry {
    pub(crate) id: String,
    pub(crate) display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) claude_slot: Option<ClaudeModelSlot>,
    #[serde(default, rename = "context1m")]
    pub(crate) context_1m: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelAssignment {
    pub(crate) account_id: String,
    pub(crate) account_alias: String,
    pub(crate) default_model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelProfile {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) models: Vec<ModelEntry>,
    pub(crate) assignments: Vec<ModelAssignment>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelayModelOption {
    pub(crate) id: String,
    pub(crate) display_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelProfilesStatus {
    pub(crate) profiles: Vec<ModelProfile>,
    pub(crate) relay_accounts: Vec<ModelAssignment>,
}
