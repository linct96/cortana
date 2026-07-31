use super::{
    antigravity::list_antigravity_sessions,
    claude::list_claude_sessions,
    codex::{list_codex_sessions, mutate_codex_session},
    grok::{delete_grok_session, list_grok_sessions},
    types::{SessionCapabilities, SessionPage, SessionSummary},
};
use crate::platform::state::{AccountProduct, AppState};
use chrono::DateTime;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use tauri::State;
use url::Url;
use uuid::Uuid;

pub(super) const PAGE_SIZE: usize = 50;

pub(crate) async fn list_sessions(
    state: State<'_, AppState>,
    product: AccountProduct,
    cursor: Option<String>,
    archived: bool,
    search_term: Option<String>,
) -> Result<SessionPage, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match product {
        AccountProduct::Codex => list_codex_sessions(&state, cursor, archived, search_term),
        AccountProduct::Claude => {
            reject_archived(archived, product)?;
            paginate_sessions(
                list_claude_sessions(&state)?,
                cursor,
                search_term,
                capabilities(product),
            )
        }
        AccountProduct::Grok => {
            reject_archived(archived, product)?;
            paginate_sessions(
                list_grok_sessions(&state)?,
                cursor,
                search_term,
                capabilities(product),
            )
        }
        AccountProduct::Antigravity => {
            reject_archived(archived, product)?;
            paginate_sessions(
                list_antigravity_sessions(&state)?,
                cursor,
                search_term,
                capabilities(product),
            )
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) async fn rename_session(
    state: State<'_, AppState>,
    product: AccountProduct,
    session_id: String,
    name: String,
) -> Result<(), String> {
    if product != AccountProduct::Codex {
        return Err(unsupported_action(product, "重命名"));
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("会话名称不能为空。".to_string());
    }
    mutate_codex_session(
        state,
        session_id,
        "thread/name/set",
        json!({ "name": name }),
    )
    .await
}

pub(crate) async fn archive_session(
    state: State<'_, AppState>,
    product: AccountProduct,
    session_id: String,
) -> Result<(), String> {
    if product != AccountProduct::Codex {
        return Err(unsupported_action(product, "归档"));
    }
    mutate_codex_session(state, session_id, "thread/archive", json!({})).await
}

pub(crate) async fn unarchive_session(
    state: State<'_, AppState>,
    product: AccountProduct,
    session_id: String,
) -> Result<(), String> {
    if product != AccountProduct::Codex {
        return Err(unsupported_action(product, "恢复"));
    }
    mutate_codex_session(state, session_id, "thread/unarchive", json!({})).await
}

pub(crate) async fn delete_session(
    state: State<'_, AppState>,
    product: AccountProduct,
    session_id: String,
) -> Result<(), String> {
    Uuid::parse_str(&session_id).map_err(|_| "会话 ID 格式无效。".to_string())?;
    match product {
        AccountProduct::Codex => {
            mutate_codex_session(state, session_id, "thread/delete", json!({})).await
        }
        AccountProduct::Grok => {
            let state = state.inner().clone();
            tauri::async_runtime::spawn_blocking(move || delete_grok_session(&state, &session_id))
                .await
                .map_err(|error| error.to_string())?
        }
        _ => Err(unsupported_action(product, "删除")),
    }
}

pub(super) fn capabilities(product: AccountProduct) -> SessionCapabilities {
    match product {
        AccountProduct::Codex => SessionCapabilities {
            supports_archived: true,
            can_rename: true,
            can_archive: true,
            can_delete: true,
        },
        AccountProduct::Grok => SessionCapabilities {
            supports_archived: false,
            can_rename: false,
            can_archive: false,
            can_delete: true,
        },
        AccountProduct::Claude | AccountProduct::Antigravity => SessionCapabilities {
            supports_archived: false,
            can_rename: false,
            can_archive: false,
            can_delete: false,
        },
    }
}

pub(super) fn reject_archived(archived: bool, product: AccountProduct) -> Result<(), String> {
    if archived {
        Err(unsupported_action(product, "查看归档会话"))
    } else {
        Ok(())
    }
}

pub(super) fn unsupported_action(product: AccountProduct, action: &str) -> String {
    format!("{} 暂不支持会话{action}。", product.display_name())
}

pub(super) fn paginate_sessions(
    mut sessions: Vec<SessionSummary>,
    cursor: Option<String>,
    search_term: Option<String>,
    capabilities: SessionCapabilities,
) -> Result<SessionPage, String> {
    if let Some(search) = trimmed_search(search_term).map(|value| value.to_lowercase()) {
        sessions.retain(|session| {
            session
                .name
                .as_deref()
                .unwrap_or_default()
                .to_lowercase()
                .contains(&search)
                || session.preview.to_lowercase().contains(&search)
        });
    }
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let offset = cursor
        .as_deref()
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| "会话游标无效。".to_string())?;
    let end = offset.saturating_add(PAGE_SIZE).min(sessions.len());
    let page = if offset < sessions.len() {
        sessions[offset..end].to_vec()
    } else {
        Vec::new()
    };
    Ok(SessionPage {
        sessions: page,
        next_cursor: (end < sessions.len()).then(|| end.to_string()),
        capabilities,
    })
}

pub(super) fn claude_config_dir(state: &AppState) -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home(state).join(".claude"))
}

pub(super) fn user_home(state: &AppState) -> &Path {
    state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home)
}

pub(super) fn read_directories(root: &Path, label: &str) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(root).map_err(|error| format!("无法读取{label}：{error}"))?;
    Ok(entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect())
}

pub(super) fn string_field(record: &Value, fields: &[&str]) -> Option<String> {
    fields
        .iter()
        .find_map(|field| record.get(*field).and_then(Value::as_str))
        .map(str::to_string)
        .and_then(non_empty)
}

pub(super) fn message_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let parts = content.as_array()?.iter().filter_map(|part| {
        (part.get("type").and_then(Value::as_str) == Some("text"))
            .then(|| part.get("text").and_then(Value::as_str))
            .flatten()
    });
    let text = parts.collect::<Vec<_>>().join(" ");
    non_empty(text)
}

pub(super) fn compact_text(value: String) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = value.chars();
    let compact = chars.by_ref().take(500).collect::<String>();
    if chars.next().is_some() {
        format!("{compact}...")
    } else {
        compact
    }
}

pub(super) fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

pub(super) fn usable_prompt(value: String) -> Option<String> {
    let value = value.trim();
    (!value.starts_with("<local-command-") && !value.starts_with("<system-reminder>"))
        .then(|| value.to_string())
}

pub(super) fn trimmed_search(value: Option<String>) -> Option<String> {
    value.and_then(non_empty)
}

pub(super) fn parse_timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

pub(super) fn parse_antigravity_timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f%:z")
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

pub(super) fn file_modified_at(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

pub(super) fn workspace_path(value: &str) -> Option<String> {
    let uri = serde_json::from_str::<Vec<String>>(value)
        .ok()?
        .into_iter()
        .next()?;
    Url::parse(&uri)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .map(|path| path.display().to_string())
}
