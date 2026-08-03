use super::{oauth_progress_snapshot, open_web_access, set_web_access_settings};
use crate::{
    features::{
        accounts::{self, oauth},
        analytics, billing, environment as env, models, prompts as agents, sessions,
    },
    products::{antigravity, claude, codex, grok},
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tauri::Manager;

pub(super) async fn dispatch_command(
    app: tauri::AppHandle,
    command: &str,
    args: Value,
) -> Result<Value, String> {
    macro_rules! result {
        ($value:expr) => {
            serde_json::to_value($value?).map_err(|error| error.to_string())
        };
    }
    match command {
        "get_app_status" => result!(
            accounts::get_app_status(app.clone(), app.state(), arg(&args, "product")?).await
        ),
        "get_codex_gateway_mode" => result!(accounts::get_codex_gateway_mode(app.state())),
        "set_codex_gateway_mode" => result!(
            accounts::set_codex_gateway_mode(
                app.state(),
                arg(&args, "enabled")?,
                optional(&args, "profileId")?
            )
            .await
        ),
        "get_terminal_app" => result!(env::get_terminal_app(app.state())),
        "set_terminal_app" => result!(env::set_terminal_app(
            app.state(),
            arg(&args, "terminalApp")?
        )),
        "switch_profile" => result!(
            accounts::switch_profile(
                app.clone(),
                app.state(),
                arg(&args, "profileId")?,
                arg(&args, "force")?,
                arg(&args, "product")?
            )
            .await
        ),
        "set_grok_relay_enabled" => result!(
            accounts::set_grok_relay_enabled(
                app.clone(),
                app.state(),
                arg(&args, "profileId")?,
                arg(&args, "enabled")?,
                arg(&args, "force")?
            )
            .await
        ),
        "open_codex_cli_with_profile" => result!(
            accounts::open_codex_cli_with_profile(app.state(), arg(&args, "profileId")?).await
        ),
        "start_oauth_add" => result!(
            oauth::start_oauth_add(
                app.clone(),
                app.state(),
                optional(&args, "alias")?,
                arg(&args, "activate")?,
                arg(&args, "product")?
            )
            .await
        ),
        "open_oauth_add" => result!(oauth::open_oauth_add(
            app.clone(),
            app.state(),
            arg(&args, "authorizationUrl")?
        )),
        "update_oauth_alias" => {
            result!(oauth::update_oauth_alias(app.state(), arg(&args, "alias")?))
        }
        "complete_oauth_add" => result!(
            oauth::complete_oauth_add(app.clone(), app.state(), arg(&args, "callbackUrl")?).await
        ),
        "cancel_oauth_add" => result!(oauth::cancel_oauth_add(app.clone(), app.state())),
        "import_current_profile" => result!(
            accounts::import_current_profile(
                app.clone(),
                app.state(),
                optional(&args, "alias")?,
                arg(&args, "product")?
            )
            .await
        ),
        "import_auth_json" => result!(
            oauth::import_auth_json(
                app.clone(),
                app.state(),
                arg(&args, "authJson")?,
                optional(&args, "alias")?,
                arg(&args, "activate")?
            )
            .await
        ),
        "add_relay_profile" => result!(accounts::add_relay_profile(
            app.clone(),
            app.state(),
            arg(&args, "apiKey")?,
            arg(&args, "apiBaseUrl")?,
            optional(&args, "alias")?,
            arg(&args, "activate")?,
            arg(&args, "product")?,
            optional(&args, "modelProfileId")?,
            optional(&args, "defaultModelId")?,
            optional(&args, "upstreamProtocol")?,
            optional(&args, "upstreamAuthMode")?,
            optional(&args, "anthropicMaxTokens")?
        )),
        "refresh_profile_usage" => {
            result!(accounts::refresh_profile_usage(app.state(), arg(&args, "profileId")?).await)
        }
        "refresh_due_profile_usage" => result!(
            accounts::refresh_due_profile_usage(app.state(), arg(&args, "immediate")?).await
        ),
        "get_usage_refresh_settings" => {
            result!(accounts::get_usage_refresh_settings(app.state()))
        }
        "set_usage_refresh_settings" => result!(accounts::set_usage_refresh_settings(
            app.state(),
            arg(&args, "enabled")?,
            arg(&args, "activeIntervalMinutes")?,
            arg(&args, "inactiveIntervalMinutes")?
        )),
        "get_profile_reset_credits" => result!(
            accounts::get_profile_reset_credits(app.state(), arg(&args, "profileId")?).await
        ),
        "consume_profile_reset_credit" => result!(
            accounts::consume_profile_reset_credit(
                app.state(),
                arg(&args, "profileId")?,
                arg(&args, "creditId")?,
                arg(&args, "idempotencyKey")?
            )
            .await
        ),
        "get_profile_auth" => result!(accounts::get_profile_auth(
            app.state(),
            arg(&args, "profileId")?,
            arg(&args, "product")?
        )),
        "get_relay_api_key" => result!(accounts::get_relay_api_key(
            app.state(),
            arg(&args, "profileId")?,
            arg(&args, "product")?
        )),
        "update_profile" => result!(accounts::update_profile(
            app.clone(),
            app.state(),
            arg(&args, "profileId")?,
            arg(&args, "alias")?,
            optional(&args, "authJson")?,
            arg(&args, "product")?
        )),
        "update_relay_profile" => result!(accounts::update_relay_profile(
            app.clone(),
            app.state(),
            arg(&args, "profileId")?,
            arg(&args, "alias")?,
            optional(&args, "apiKey")?,
            arg(&args, "apiBaseUrl")?,
            arg(&args, "product")?,
            optional(&args, "modelProfileId")?,
            optional(&args, "defaultModelId")?,
            arg(&args, "force")?,
            optional(&args, "upstreamProtocol")?,
            optional(&args, "upstreamAuthMode")?,
            optional(&args, "anthropicMaxTokens")?
        )),
        "get_model_profiles_status" => {
            result!(models::get_model_profiles_status(app.state(), arg(&args, "product")?).await)
        }
        "create_model_profile" => result!(models::create_model_profile(
            app.state(),
            arg(&args, "product")?,
            arg(&args, "name")?,
            arg(&args, "models")?,
            arg(&args, "assignments")?,
            arg(&args, "forceReassign")?
        )),
        "update_model_profile" => result!(models::update_model_profile(
            app.state(),
            arg(&args, "product")?,
            arg(&args, "profileId")?,
            arg(&args, "name")?,
            arg(&args, "models")?,
            arg(&args, "assignments")?,
            arg(&args, "forceReassign")?
        )),
        "delete_model_profile" => result!(models::delete_model_profile(
            app.state(),
            arg(&args, "product")?,
            arg(&args, "profileId")?
        )),
        "fetch_relay_models" => {
            result!(
                models::fetch_relay_models(
                    app.state(),
                    arg(&args, "product")?,
                    arg(&args, "accountId")?
                )
                .await
            )
        }
        "reorder_profiles" => result!(accounts::reorder_profiles(
            app.clone(),
            app.state(),
            arg(&args, "profileIds")?,
            arg(&args, "product")?
        )),
        "delete_profile" => result!(accounts::delete_profile(
            app.clone(),
            app.state(),
            arg(&args, "profileId")?,
            arg(&args, "product")?
        )),
        "get_active_product" => result!(accounts::get_active_product(app.state())),
        "set_active_product" => result!(accounts::set_active_product(
            app.clone(),
            app.state(),
            arg(&args, "product")?
        )),
        "set_codex_home" => result!(accounts::set_codex_home(
            app.clone(),
            app.state(),
            arg(&args, "codexHome")?
        )),
        "get_agents_status" => result!(agents::get_agents_status(
            app.state(),
            arg(&args, "product")?
        )),
        "create_agents_profile" => result!(agents::create_agents_profile(
            app.state(),
            arg(&args, "product")?,
            arg(&args, "name")?,
            arg(&args, "content")?
        )),
        "update_agents_profile" => result!(agents::update_agents_profile(
            app.state(),
            arg(&args, "product")?,
            arg(&args, "profileId")?,
            arg(&args, "name")?,
            arg(&args, "content")?
        )),
        "activate_agents_profile" => result!(agents::activate_agents_profile(
            app.state(),
            arg(&args, "product")?,
            arg(&args, "profileId")?,
            arg(&args, "force")?
        )),
        "import_current_agents" => result!(agents::import_current_agents(
            app.state(),
            arg(&args, "product")?,
            arg(&args, "name")?
        )),
        "delete_agents_profile" => result!(agents::delete_agents_profile(
            app.state(),
            arg(&args, "product")?,
            arg(&args, "profileId")?
        )),
        "get_usage_analytics" => {
            result!(
                analytics::get_usage_analytics(
                    app.state(),
                    arg(&args, "product")?,
                    arg(&args, "range")?
                )
                .await
            )
        }
        "list_model_pricing" => result!(billing::list_model_pricing(app.state())),
        "save_model_pricing" => result!(billing::save_model_pricing(
            app.state(),
            arg(&args, "items")?
        )),
        "delete_model_pricing" => result!(billing::delete_model_pricing(
            app.state(),
            arg(&args, "modelId")?
        )),
        "fetch_models_dev_pricing" => result!(billing::fetch_models_dev_pricing().await),
        "get_codex_config" => result!(codex::get_codex_config(app.state())),
        "validate_codex_config" => result!(Ok::<_, String>(codex::validate_codex_config(arg(
            &args, "content"
        )?))),
        "format_codex_config" => result!(codex::format_codex_config(arg(&args, "content")?)),
        "save_codex_config" => result!(codex::save_codex_config(
            app.state(),
            arg(&args, "content")?
        )),
        "get_claude_config" => result!(claude::get_claude_config(app.state())),
        "validate_claude_config" => result!(Ok::<_, String>(claude::validate_claude_config(arg(
            &args, "content"
        )?))),
        "format_claude_config" => result!(claude::format_claude_config(arg(&args, "content")?)),
        "save_claude_config" => result!(claude::save_claude_config(
            app.state(),
            arg(&args, "content")?
        )),
        "get_antigravity_config" => result!(antigravity::get_antigravity_config(app.state())),
        "validate_antigravity_config" => result!(Ok::<_, String>(
            antigravity::validate_antigravity_config(arg(&args, "content")?)
        )),
        "format_antigravity_config" => result!(antigravity::format_antigravity_config(arg(
            &args, "content"
        )?)),
        "save_antigravity_config" => result!(antigravity::save_antigravity_config(
            app.state(),
            arg(&args, "content")?
        )),
        "get_grok_config" => result!(grok::get_grok_config(app.state())),
        "validate_grok_config" => result!(Ok::<_, String>(grok::validate_grok_config(arg(
            &args, "content"
        )?))),
        "format_grok_config" => result!(grok::format_grok_config(arg(&args, "content")?)),
        "save_grok_config" => result!(grok::save_grok_config(app.state(), arg(&args, "content")?)),
        "is_codex_cli_available" => result!(env::is_codex_cli_available(app.state()).await),
        "is_claude_cli_available" => result!(env::is_claude_cli_available(app.state()).await),
        "is_antigravity_cli_available" => {
            result!(env::is_antigravity_cli_available(app.state()).await)
        }
        "is_grok_cli_available" => result!(env::is_grok_cli_available(app.state()).await),
        "get_codex_cli_environment" => result!(env::get_codex_cli_environment(app.state()).await),
        "get_claude_cli_environment" => {
            result!(env::get_claude_cli_environment(app.state()).await)
        }
        "get_antigravity_cli_environment" => {
            result!(env::get_antigravity_cli_environment(app.state()).await)
        }
        "get_grok_cli_environment" => {
            result!(env::get_grok_cli_environment(app.state()).await)
        }
        "list_sessions" => result!(
            sessions::list_sessions(
                app.state(),
                arg(&args, "product")?,
                optional(&args, "cursor")?,
                arg(&args, "archived")?,
                optional(&args, "searchTerm")?
            )
            .await
        ),
        "rename_session" => result!(
            sessions::rename_session(
                app.state(),
                arg(&args, "product")?,
                arg(&args, "sessionId")?,
                arg(&args, "name")?
            )
            .await
        ),
        "archive_session" => {
            result!(
                sessions::archive_session(
                    app.state(),
                    arg(&args, "product")?,
                    arg(&args, "sessionId")?
                )
                .await
            )
        }
        "unarchive_session" => {
            result!(
                sessions::unarchive_session(
                    app.state(),
                    arg(&args, "product")?,
                    arg(&args, "sessionId")?
                )
                .await
            )
        }
        "delete_session" => {
            result!(
                sessions::delete_session(
                    app.state(),
                    arg(&args, "product")?,
                    arg(&args, "sessionId")?
                )
                .await
            )
        }
        "set_autostart" => result!(crate::platform::shell::set_autostart(
            app.clone(),
            arg(&args, "enabled")?
        )),
        "reveal_data_directory" => {
            result!(crate::platform::shell::reveal_data_directory(
                app.clone(),
                app.state()
            ))
        }
        "open_codex_home" => result!(crate::platform::shell::open_codex_home(
            app.clone(),
            arg(&args, "codexHome")?
        )),
        "open_codex_cli_install_page" => {
            result!(crate::platform::shell::open_codex_cli_install_page(
                app.clone()
            ))
        }
        "open_claude_cli_install_page" => {
            result!(crate::platform::shell::open_claude_cli_install_page(
                app.clone()
            ))
        }
        "open_antigravity_cli_install_page" => {
            result!(crate::platform::shell::open_antigravity_cli_install_page(
                app.clone()
            ))
        }
        "open_grok_cli_install_page" => result!(
            crate::platform::shell::open_grok_cli_install_page(app.clone())
        ),
        "open_web_access" => result!(open_web_access(app.clone())),
        "set_web_access_settings" => result!(set_web_access_settings(
            app.clone(),
            arg(&args, "enabled")?,
            arg(&args, "port")?
        )),
        "get_oauth_progress" => result!(Ok::<_, String>(oauth_progress_snapshot(&app)?)),
        _ => Err(format!("未知命令：{command}")),
    }
}

pub(super) fn arg<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
    args.get(key)
        .cloned()
        .ok_or_else(|| format!("缺少参数：{key}"))
        .and_then(|value| serde_json::from_value(value).map_err(|_| format!("参数格式无效：{key}")))
}

pub(super) fn optional<T: DeserializeOwned>(args: &Value, key: &str) -> Result<Option<T>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| format!("参数格式无效：{key}")),
    }
}
