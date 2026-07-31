use super::{
    antigravity::list_antigravity_sessions_at,
    codex::decode_response,
    commands::{capabilities, paginate_sessions, parse_antigravity_timestamp, workspace_path},
    grok::{grok_session_summary, GrokSessionFile},
};
use crate::platform::state::AccountProduct;
use rusqlite::{params, Connection};
use serde_json::json;
use std::fs;
use url::Url;
use uuid::Uuid;

#[test]
fn decodes_matching_app_server_responses() {
    assert!(
        decode_response(r#"{"method":"thread/started","params":{}}"#)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        decode_response(r#"{"id":1,"result":{"data":[]}}"#).unwrap(),
        Some(json!({ "data": [] }))
    );
    assert_eq!(
        decode_response(r#"{"id":1,"error":{"message":"missing"}}"#).unwrap_err(),
        "Codex app-server 请求失败：missing"
    );
}

#[test]
fn parses_grok_summary_and_paginates() {
    let id = Uuid::new_v4().to_string();
    let summary: GrokSessionFile = serde_json::from_value(json!({
        "info": { "id": id, "cwd": "/tmp/project" },
        "session_summary": "preview",
        "generated_title": "title",
        "created_at": "2026-07-01T00:00:00Z",
        "updated_at": "2026-07-02T00:00:00Z"
    }))
    .unwrap();
    let session = grok_session_summary(summary, 0).unwrap();
    let page = paginate_sessions(
        vec![session],
        None,
        Some("title".to_string()),
        capabilities(AccountProduct::Grok),
    )
    .unwrap();
    assert_eq!(page.sessions.len(), 1);
    assert!(page.capabilities.can_delete);
    assert!(!page.capabilities.can_rename);
}

#[test]
fn parses_antigravity_workspace_and_timestamp() {
    let project = std::env::temp_dir().join("project");
    let uri = serde_json::to_string(&vec![Url::from_file_path(&project).unwrap()]).unwrap();
    assert_eq!(workspace_path(&uri).as_deref(), project.to_str());
    assert!(parse_antigravity_timestamp("2026-07-15 03:30:03.720555+00:00").unwrap() > 0);
}

#[test]
fn reads_only_top_level_antigravity_sessions() {
    let root = std::env::temp_dir().join(format!("cortana-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("sessions.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE conversation_summaries (
                    conversation_id TEXT,
                    title TEXT,
                    preview TEXT,
                    last_modified_time TEXT,
                    workspace_uris TEXT,
                    source TEXT,
                    nesting_depth INTEGER,
                    killed INTEGER
                 );",
        )
        .unwrap();
    let project = std::env::temp_dir().join("project");
    let uri = serde_json::to_string(&vec![Url::from_file_path(&project).unwrap()]).unwrap();
    connection
        .execute(
            "INSERT INTO conversation_summaries VALUES
                    ('top', '', 'Visible', '2026-07-15 03:30:03+00:00', ?1, '', 0, 0),
                    ('nested', '', 'Hidden', '2026-07-15 03:30:03+00:00', '[]', '', 1, 0)",
            params![uri],
        )
        .unwrap();
    drop(connection);

    let sessions = list_antigravity_sessions_at(&path).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "top");
    assert_eq!(sessions[0].cwd.as_deref(), project.to_str());
    fs::remove_dir_all(root).unwrap();
}
