use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

pub(crate) const OAUTH_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub(crate) const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub(crate) const OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(crate) const OAUTH_CALLBACK_URL: &str = "http://localhost:1455/auth/callback";
pub(crate) const OAUTH_SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
pub(crate) const ANTIGRAVITY_OAUTH_AUTHORIZE_URL: &str =
    "https://accounts.google.com/o/oauth2/v2/auth";
pub(crate) const ANTIGRAVITY_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub(crate) const ANTIGRAVITY_OAUTH_USERINFO_URL: &str =
    "https://www.googleapis.com/oauth2/v2/userinfo";
pub(crate) const ANTIGRAVITY_OAUTH_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
pub(crate) const ANTIGRAVITY_OAUTH_CLIENT_SECRET: &str = "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";
pub(crate) const ANTIGRAVITY_OAUTH_SCOPE: &str = "openid https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";
pub(crate) const OAUTH_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub(crate) const MAX_IMPORTED_AUTH_JSON_BYTES: usize = 1024 * 1024;
pub(crate) const ACCOUNT_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
pub(crate) const RESET_CREDITS_URL: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
pub(crate) const RESET_CREDITS_CONSUME_URL: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume";
pub(crate) const TRAY_ID: &str = "account-switcher";
pub(crate) const ACCOUNT_TYPE_OAUTH: &str = "oauth";
pub(crate) const ACCOUNT_TYPE_RELAY: &str = "relay";
pub(crate) const CORTANA_MODEL_PROVIDER: &str = "cortana";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AccountProduct {
    Codex,
    Claude,
    Antigravity,
    Grok,
}

impl AccountProduct {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Antigravity => "antigravity",
            Self::Grok => "grok",
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
            Self::Antigravity => "Antigravity",
            Self::Grok => "Grok",
        }
    }
}

pub(crate) fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) database_path: PathBuf,
    pub(crate) default_codex_home: PathBuf,
    pub(crate) pending_oauth: Arc<Mutex<Option<PendingOAuth>>>,
}

#[derive(Clone)]
pub(crate) struct PendingOAuth {
    pub(crate) product: AccountProduct,
    pub(crate) alias: String,
    pub(crate) activate: bool,
    pub(crate) code_verifier: String,
    pub(crate) state: String,
    pub(crate) callback_url: String,
    pub(crate) exchanging: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProfileSummary {
    pub(crate) id: String,
    pub(crate) product: AccountProduct,
    pub(crate) account_type: String,
    pub(crate) api_base_url: Option<String>,
    pub(crate) account_id: String,
    pub(crate) email: String,
    pub(crate) alias: String,
    pub(crate) plan_type: String,
    pub(crate) usage_primary: Option<UsageWindow>,
    pub(crate) usage_secondary: Option<UsageWindow>,
    pub(crate) antigravity_quota: Option<AntigravityQuota>,
    pub(crate) usage_updated_at: Option<i64>,
    pub(crate) reset_credits_available_count: Option<i64>,
    pub(crate) needs_reauthorization: bool,
    pub(crate) is_renewable: bool,
    pub(crate) is_active: bool,
    pub(crate) last_used_at: Option<i64>,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageRefreshSettings {
    pub(crate) enabled: bool,
    pub(crate) active_interval_minutes: u64,
    pub(crate) inactive_interval_minutes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageRefreshResult {
    pub(crate) profile: ProfileSummary,
    pub(crate) refreshed: bool,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageRefreshRunResult {
    pub(crate) refreshed_count: usize,
    pub(crate) skipped_count: usize,
    pub(crate) failed_count: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AntigravityQuota {
    pub(crate) project_id: Option<String>,
    #[serde(default)]
    pub(crate) forbidden: bool,
    #[serde(default)]
    pub(crate) models: Vec<AntigravityModelQuota>,
    #[serde(default)]
    pub(crate) groups: Vec<AntigravityQuotaGroup>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AntigravityModelQuota {
    pub(crate) model_id: String,
    pub(crate) display_name: String,
    pub(crate) remaining_percent: i64,
    pub(crate) resets_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AntigravityQuotaGroup {
    pub(crate) display_name: String,
    pub(crate) buckets: Vec<AntigravityQuotaBucket>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AntigravityQuotaBucket {
    pub(crate) bucket_id: String,
    pub(crate) window: String,
    pub(crate) display_name: String,
    pub(crate) remaining_percent: i64,
    pub(crate) resets_at: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResetCredits {
    pub(crate) available_count: i64,
    pub(crate) credits: Vec<ResetCredit>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResetCredit {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) expires_at: String,
    pub(crate) granted_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ResetCreditConsumeOutcome {
    Reset,
    AlreadyRedeemed,
    NothingToReset,
    NoCredit,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResetCreditConsumeResult {
    pub(crate) outcome: ResetCreditConsumeOutcome,
    pub(crate) profile: ProfileSummary,
    pub(crate) credits: ResetCredits,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageWindow {
    pub(crate) used_percent: f64,
    pub(crate) window_minutes: Option<i64>,
    pub(crate) resets_at: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppStatus {
    pub(crate) profiles: Vec<ProfileSummary>,
    pub(crate) detected_profile: Option<ProfileSummary>,
    pub(crate) auth_path: String,
    pub(crate) auth_state: AuthState,
    pub(crate) autostart_enabled: bool,
    pub(crate) web_access: WebAccessStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WebAccessStatus {
    pub(crate) enabled: bool,
    pub(crate) port: u16,
    pub(crate) available: bool,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigFile {
    pub(crate) path: String,
    pub(crate) content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ConfigDiagnostic {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthState {
    pub(crate) kind: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OAuthProgress {
    pub(crate) stage: String,
    pub(crate) message: String,
    pub(crate) profile: Option<ProfileSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OAuthProgressSnapshot {
    pub(crate) sequence: u64,
    pub(crate) pending: bool,
    pub(crate) progress: Option<OAuthProgress>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OAuthTokenResponse {
    pub(crate) access_token: Option<String>,
    pub(crate) refresh_token: Option<String>,
    pub(crate) id_token: Option<String>,
    pub(crate) expires_in: Option<i64>,
    pub(crate) token_type: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Identity {
    pub(crate) account_id: String,
    pub(crate) name: String,
    pub(crate) email: String,
    pub(crate) plan_type: String,
}
