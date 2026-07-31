use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliEnvironment {
    pub(super) installed: bool,
    pub(super) installed_version: Option<String>,
    pub(super) latest_version: Option<String>,
    pub(super) install_method: String,
}
