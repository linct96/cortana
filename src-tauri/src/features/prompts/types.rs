use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentsProfile {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) content: String,
    pub(super) is_active: bool,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentsStatus {
    pub(super) profiles: Vec<AgentsProfile>,
    pub(super) path: String,
    pub(super) file_state: String,
    pub(super) unmanaged_content: Option<String>,
}
