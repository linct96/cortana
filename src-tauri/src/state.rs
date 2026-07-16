use super::*;

pub(super) const OAUTH_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub(super) const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub(super) const OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(super) const OAUTH_CALLBACK_URL: &str = "http://localhost:1455/auth/callback";
pub(super) const OAUTH_SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
pub(super) const OAUTH_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub(super) const MAX_IMPORTED_AUTH_JSON_BYTES: usize = 1024 * 1024;
pub(super) const ACCOUNT_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
pub(super) const RESET_CREDITS_URL: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
pub(super) const TRAY_ID: &str = "account-switcher";
pub(super) const ACCOUNT_TYPE_OAUTH: &str = "oauth";
pub(super) const ACCOUNT_TYPE_RELAY: &str = "relay";
pub(super) const RELAY_MODEL_PROVIDER: &str = "relay";

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
    pub(super) alias: String,
    pub(super) activate: bool,
    pub(super) code_verifier: String,
    pub(super) state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProfileSummary {
    pub(super) id: String,
    pub(super) account_type: String,
    pub(super) api_base_url: Option<String>,
    pub(super) account_id: String,
    pub(super) email: String,
    pub(super) alias: String,
    pub(super) plan_type: String,
    pub(super) usage_primary: Option<UsageWindow>,
    pub(super) usage_secondary: Option<UsageWindow>,
    pub(super) usage_updated_at: Option<i64>,
    pub(super) reset_credits_available_count: Option<i64>,
    pub(super) is_active: bool,
    pub(super) last_used_at: Option<i64>,
    pub(super) updated_at: i64,
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
pub(super) struct CodexConfigFile {
    pub(super) path: String,
    pub(super) content: String,
}

#[derive(Debug, Serialize)]
pub(super) struct CodexConfigDiagnostic {
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
}

#[derive(Debug, Default)]
pub(super) struct Identity {
    pub(super) account_id: String,
    pub(super) email: String,
    pub(super) plan_type: String,
}
