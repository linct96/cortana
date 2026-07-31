use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionSummary {
    pub(super) id: String,
    pub(super) name: Option<String>,
    pub(super) preview: String,
    pub(super) cwd: Option<String>,
    pub(super) source: Option<String>,
    pub(super) created_at: Option<i64>,
    pub(super) updated_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionCapabilities {
    pub(super) supports_archived: bool,
    pub(super) can_rename: bool,
    pub(super) can_archive: bool,
    pub(super) can_delete: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionPage {
    pub(super) sessions: Vec<SessionSummary>,
    pub(super) next_cursor: Option<String>,
    pub(super) capabilities: SessionCapabilities,
}
