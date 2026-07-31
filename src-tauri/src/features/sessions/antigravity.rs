use super::{
    commands::{compact_text, non_empty, parse_antigravity_timestamp, user_home, workspace_path},
    types::SessionSummary,
};
use crate::platform::state::AppState;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

pub(super) fn list_antigravity_sessions(state: &AppState) -> Result<Vec<SessionSummary>, String> {
    let path = user_home(state).join(".gemini/antigravity-cli/conversation_summaries.db");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    list_antigravity_sessions_at(&path)
}

pub(super) fn list_antigravity_sessions_at(path: &Path) -> Result<Vec<SessionSummary>, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("无法只读打开 Antigravity 会话索引：{error}"))?;
    let mut statement = connection
        .prepare(
            "SELECT conversation_id, COALESCE(title, ''), COALESCE(preview, ''),
                    last_modified_time, COALESCE(workspace_uris, '[]'), COALESCE(source, '')
             FROM conversation_summaries
             WHERE nesting_depth = 0 AND killed = 0",
        )
        .map_err(|error| format!("Antigravity 会话索引格式不兼容：{error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|error| format!("无法查询 Antigravity 会话索引：{error}"))?;

    let mut sessions = Vec::new();
    for row in rows.flatten() {
        let Some(updated_at) = parse_antigravity_timestamp(&row.3).filter(|value| *value > 0)
        else {
            continue;
        };
        sessions.push(SessionSummary {
            id: row.0,
            name: non_empty(row.1).map(compact_text),
            preview: compact_text(row.2),
            cwd: workspace_path(&row.4),
            source: non_empty(row.5).or_else(|| Some("cli".to_string())),
            created_at: None,
            updated_at,
        });
    }
    Ok(sessions)
}
