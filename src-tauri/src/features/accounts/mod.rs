mod commands;
pub(crate) mod oauth;
mod store;

pub(crate) use crate::products::codex::{
    consume_profile_reset_credit, get_profile_reset_credits, start_usage_refresh_scheduler,
};
pub(crate) use commands::set_account_usage_refresh_settings as set_usage_refresh_settings;
pub(crate) use commands::{
    active_product, add_relay_profile, delete_profile, get_active_product, get_app_status,
    get_codex_gateway_mode, get_profile_auth, get_relay_api_key, get_usage_refresh_settings,
    import_current_profile, open_codex_cli_with_profile, refresh_due_profile_usage,
    refresh_profile_usage, reorder_profiles, set_active_product, set_codex_gateway_mode,
    set_codex_home, set_grok_relay_enabled, switch_profile, update_profile, update_relay_profile,
};
#[cfg(test)]
pub(crate) use store::{get_profile_auth_json, upsert_relay_profile};
pub(crate) use store::{
    get_profile_summary, get_profile_summary_for_product, list_profiles_for_product, relay_alias,
    relay_api_key_for_profile, resolve_auth_state,
};

#[cfg(test)]
mod tests;
