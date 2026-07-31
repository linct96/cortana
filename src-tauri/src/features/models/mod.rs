mod claude;
mod codex;
mod commands;
mod grok;
mod remote;
mod store;
mod types;

pub(crate) use claude::apply_claude_model_config;
pub(crate) use codex::apply_model_config;
pub(crate) use commands::{
    create_model_profile, delete_model_profile, fetch_relay_models, get_model_profiles_status,
    update_model_profile,
};
pub(crate) use grok::{
    apply_grok_model_config, grok_config_matches_accounts, infer_grok_enabled_accounts,
};
#[cfg(test)]
pub(crate) use store::save_model_profile;
pub(crate) use store::{set_account_model_profile, validate_model_selection};
#[cfg(test)]
pub(crate) use types::{ModelAssignment, ModelEntry};

#[cfg(test)]
mod tests;
