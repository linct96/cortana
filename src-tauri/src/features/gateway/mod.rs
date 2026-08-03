use crate::platform::{
    db::{get_setting, open_database, set_setting},
    state::AppState,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rusqlite::Connection;
use std::sync::atomic::{AtomicBool, Ordering};

mod anthropic_messages;
mod logging;
mod openai_chat;
mod openai_responses;
mod server;
mod types;

pub(crate) use server::initialize;
pub(crate) use types::{
    GatewayStatus, UpstreamAuthMode, UpstreamProtocol, DEFAULT_ANTHROPIC_MAX_TOKENS,
};

pub(crate) const GATEWAY_PORT: u16 = 11_457;
pub(crate) const GATEWAY_ENABLED_SETTING: &str = "codex_gateway_enabled";
pub(crate) const GATEWAY_API_KEY_SETTING: &str = "codex_gateway_api_key";
pub(crate) const ACTIVE_PROFILE_SETTING: &str = "active_codex_profile_id";
static GATEWAY_AVAILABLE: AtomicBool = AtomicBool::new(false);

pub(crate) fn base_url() -> String {
    format!("http://127.0.0.1:{GATEWAY_PORT}/v1")
}

pub(crate) fn is_base_url(value: &str) -> bool {
    value.trim_end_matches('/') == base_url()
}

pub(crate) fn is_enabled(connection: &Connection) -> Result<bool, String> {
    Ok(get_setting(connection, GATEWAY_ENABLED_SETTING)?.as_deref() == Some("true"))
}

pub(crate) fn ensure_available() -> Result<(), String> {
    if !GATEWAY_AVAILABLE.load(Ordering::Acquire) {
        return Err(format!(
            "Codex 网关未运行，请确认 127.0.0.1:{GATEWAY_PORT} 端口可用并重启 Cortana。"
        ));
    }
    Ok(())
}

pub(crate) fn gateway_status(state: &AppState) -> Result<GatewayStatus, String> {
    let connection = open_database(state)?;
    Ok(GatewayStatus {
        enabled: is_enabled(&connection)?,
        available: GATEWAY_AVAILABLE.load(Ordering::Acquire),
        active_profile_id: get_setting(&connection, ACTIVE_PROFILE_SETTING)?,
    })
}

pub(crate) fn local_api_key(connection: &Connection) -> Result<Option<String>, String> {
    get_setting(connection, GATEWAY_API_KEY_SETTING)
}

pub(crate) fn ensure_local_api_key(connection: &Connection) -> Result<String, String> {
    if let Some(key) = local_api_key(connection)?.filter(|key| !key.trim().is_empty()) {
        return Ok(key);
    }
    let mut bytes = [0_u8; 32];
    rand::fill(&mut bytes);
    let key = format!("cortana-gw-{}", URL_SAFE_NO_PAD.encode(bytes));
    set_setting(connection, GATEWAY_API_KEY_SETTING, &key)?;
    Ok(key)
}

fn set_available(available: bool) {
    GATEWAY_AVAILABLE.store(available, Ordering::Release);
}

#[cfg(test)]
pub(crate) fn mark_available_for_test() {
    set_available(true);
}

#[cfg(test)]
mod tests {
    use super::{base_url, is_base_url};

    #[test]
    fn accepts_only_the_fixed_gateway_base_url() {
        assert!(is_base_url(&base_url()));
        assert!(is_base_url(&(base_url() + "/")));
        assert!(!is_base_url("http://localhost:11457/v1"));
    }
}
