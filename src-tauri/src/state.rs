use super::*;

pub(super) const OAUTH_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub(super) const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub(super) const OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(super) const OAUTH_CALLBACK_URL: &str = "http://localhost:1455/auth/callback";
pub(super) const OAUTH_SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
pub(super) const ANTIGRAVITY_OAUTH_AUTHORIZE_URL: &str =
    "https://accounts.google.com/o/oauth2/v2/auth";
pub(super) const ANTIGRAVITY_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub(super) const ANTIGRAVITY_OAUTH_USERINFO_URL: &str =
    "https://www.googleapis.com/oauth2/v2/userinfo";
pub(super) const ANTIGRAVITY_OAUTH_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
pub(super) const ANTIGRAVITY_OAUTH_CLIENT_SECRET: &str = "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";
pub(super) const ANTIGRAVITY_OAUTH_SCOPE: &str = "openid https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";
pub(super) const OAUTH_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub(super) const MAX_IMPORTED_AUTH_JSON_BYTES: usize = 1024 * 1024;
pub(super) const ACCOUNT_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
pub(super) const RESET_CREDITS_URL: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
pub(super) const TRAY_ID: &str = "account-switcher";
pub(super) const ACCOUNT_TYPE_OAUTH: &str = "oauth";
pub(super) const ACCOUNT_TYPE_RELAY: &str = "relay";
pub(super) const RELAY_MODEL_PROVIDER: &str = "relay";

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum AccountProduct {
    #[default]
    Codex,
    Claude,
    Antigravity,
    Grok,
}

impl AccountProduct {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Antigravity => "antigravity",
            Self::Grok => "grok",
        }
    }

    pub(super) fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
            Self::Antigravity => "Antigravity",
            Self::Grok => "Grok",
        }
    }
}

pub(super) fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

#[derive(Clone)]
pub(super) struct AppState {
    pub(super) database_path: PathBuf,
    pub(super) default_codex_home: PathBuf,
    pub(super) pending_oauth: Arc<Mutex<Option<PendingOAuth>>>,
}

#[derive(Clone)]
pub(super) struct PendingOAuth {
    pub(super) product: AccountProduct,
    pub(super) alias: String,
    pub(super) activate: bool,
    pub(super) code_verifier: String,
    pub(super) state: String,
    pub(super) callback_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProfileSummary {
    pub(super) id: String,
    pub(super) product: AccountProduct,
    pub(super) account_type: String,
    pub(super) api_base_url: Option<String>,
    pub(super) account_id: String,
    pub(super) email: String,
    pub(super) alias: String,
    pub(super) plan_type: String,
    pub(super) usage_primary: Option<UsageWindow>,
    pub(super) usage_secondary: Option<UsageWindow>,
    pub(super) antigravity_quota: Option<AntigravityQuota>,
    pub(super) usage_updated_at: Option<i64>,
    pub(super) reset_credits_available_count: Option<i64>,
    pub(super) is_renewable: bool,
    pub(super) is_active: bool,
    pub(super) last_used_at: Option<i64>,
    pub(super) updated_at: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UsageRefreshSettings {
    pub(super) enabled: bool,
    pub(super) active_interval_minutes: u64,
    pub(super) inactive_interval_minutes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UsageRefreshResult {
    pub(super) profile: ProfileSummary,
    pub(super) refreshed: bool,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UsageRefreshRunResult {
    pub(super) refreshed_count: usize,
    pub(super) skipped_count: usize,
    pub(super) failed_count: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AntigravityQuota {
    pub(super) project_id: Option<String>,
    #[serde(default)]
    pub(super) forbidden: bool,
    #[serde(default)]
    pub(super) models: Vec<AntigravityModelQuota>,
    #[serde(default)]
    pub(super) groups: Vec<AntigravityQuotaGroup>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AntigravityModelQuota {
    pub(super) model_id: String,
    pub(super) display_name: String,
    pub(super) remaining_percent: i64,
    pub(super) resets_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AntigravityQuotaGroup {
    pub(super) display_name: String,
    pub(super) buckets: Vec<AntigravityQuotaBucket>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AntigravityQuotaBucket {
    pub(super) bucket_id: String,
    pub(super) window: String,
    pub(super) display_name: String,
    pub(super) remaining_percent: i64,
    pub(super) resets_at: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ResetCredits {
    pub(super) available_count: i64,
    pub(super) credits: Vec<ResetCredit>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ResetCredit {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) status: String,
    pub(super) expires_at: String,
    pub(super) granted_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UsageWindow {
    pub(super) used_percent: f64,
    pub(super) window_minutes: Option<i64>,
    pub(super) resets_at: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AppStatus {
    pub(super) profiles: Vec<ProfileSummary>,
    pub(super) detected_profile: Option<ProfileSummary>,
    pub(super) auth_path: String,
    pub(super) auth_state: AuthState,
    pub(super) autostart_enabled: bool,
    pub(super) web_access: WebAccessStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WebAccessStatus {
    pub(super) enabled: bool,
    pub(super) port: u16,
    pub(super) available: bool,
    pub(super) error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConfigFile {
    pub(super) path: String,
    pub(super) content: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ConfigDiagnostic {
    pub(super) from: usize,
    pub(super) to: usize,
    pub(super) message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AuthState {
    pub(super) kind: String,
    pub(super) message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OAuthProgress {
    pub(super) stage: String,
    pub(super) message: String,
    pub(super) profile: Option<ProfileSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OAuthProgressSnapshot {
    pub(super) sequence: u64,
    pub(super) pending: bool,
    pub(super) progress: Option<OAuthProgress>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OAuthTokenResponse {
    pub(super) access_token: Option<String>,
    pub(super) refresh_token: Option<String>,
    pub(super) id_token: Option<String>,
    pub(super) expires_in: Option<i64>,
    pub(super) token_type: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct Identity {
    pub(super) account_id: String,
    pub(super) name: String,
    pub(super) email: String,
    pub(super) plan_type: String,
}
