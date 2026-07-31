use super::{
    commands::{compact_text, file_modified_at, non_empty, parse_timestamp, read_directories},
    types::SessionSummary,
};
use crate::{
    features::environment::{codex_command, grok_candidates, grok_home},
    platform::state::AppState,
};
use serde::Deserialize;
use std::fs;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub(super) struct GrokSessionInfo {
    id: String,
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GrokSessionFile {
    info: GrokSessionInfo,
    session_summary: Option<String>,
    generated_title: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

pub(super) fn list_grok_sessions(state: &AppState) -> Result<Vec<SessionSummary>, String> {
    let root = grok_home(state).join("sessions");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut sessions = Vec::new();
    for workspace in read_directories(&root, "Grok 会话目录")? {
        for session_dir in read_directories(&workspace, "Grok 会话目录").unwrap_or_default() {
            let path = session_dir.join("summary.json");
            if !path.is_file() {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(summary) = serde_json::from_str::<GrokSessionFile>(&content) else {
                continue;
            };
            if let Some(session) = grok_session_summary(summary, file_modified_at(&path)) {
                sessions.push(session);
            }
        }
    }
    Ok(sessions)
}

pub(super) fn grok_session_summary(
    summary: GrokSessionFile,
    modified_at: i64,
) -> Option<SessionSummary> {
    Uuid::parse_str(&summary.info.id).ok()?;
    let preview = compact_text(summary.session_summary.unwrap_or_default());
    Some(SessionSummary {
        id: summary.info.id,
        name: summary
            .generated_title
            .and_then(non_empty)
            .map(compact_text),
        preview,
        cwd: summary.info.cwd.and_then(non_empty),
        source: Some("cli".to_string()),
        created_at: summary.created_at.as_deref().and_then(parse_timestamp),
        updated_at: summary
            .updated_at
            .as_deref()
            .and_then(parse_timestamp)
            .unwrap_or(modified_at),
    })
}

pub(super) fn delete_grok_session(state: &AppState, session_id: &str) -> Result<(), String> {
    let mut last_error = None;
    for candidate in grok_candidates(state) {
        match codex_command(&candidate, &["sessions", "delete", session_id])
            .env("GROK_HOME", grok_home(state))
            .output()
        {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                let message = String::from_utf8_lossy(if output.stderr.is_empty() {
                    &output.stdout
                } else {
                    &output.stderr
                });
                return Err(format!("Grok 会话删除失败：{}", message.trim()));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(format!(
        "未找到可用的 Grok CLI，请先安装或更新 Grok。{}",
        last_error
            .map(|error| format!("（{error}）"))
            .unwrap_or_default()
    ))
}
