use super::{
    commands::{
        claude_config_dir, compact_text, file_modified_at, message_text, non_empty,
        parse_timestamp, read_directories, string_field, usable_prompt,
    },
    types::SessionSummary,
};
use crate::platform::state::AppState;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
};
use uuid::Uuid;

pub(super) fn list_claude_sessions(state: &AppState) -> Result<Vec<SessionSummary>, String> {
    let projects = claude_config_dir(state).join("projects");
    if !projects.is_dir() {
        return Ok(Vec::new());
    }

    let mut file_count = 0;
    let mut sessions = Vec::new();
    for project in read_directories(&projects, "Claude 会话目录")? {
        let entries = match fs::read_dir(&project) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                continue;
            }
            file_count += 1;
            if let Some(session) = parse_claude_session(&path) {
                sessions.push(session);
            }
        }
    }
    if file_count > 0 && sessions.is_empty() {
        return Err("Claude 会话文件格式不兼容，请更新 Cortana 后重试。".to_string());
    }
    Ok(sessions)
}

pub(super) fn parse_claude_session(path: &Path) -> Option<SessionSummary> {
    let id = path.file_stem()?.to_str()?.to_string();
    Uuid::parse_str(&id).ok()?;
    let file = fs::File::open(path).ok()?;
    let modified_at = file_modified_at(path);
    let mut custom_title = None;
    let mut agent_name = None;
    let mut ai_title = None;
    let mut last_prompt = None;
    let mut first_prompt = None;
    let mut cwd = None;
    let mut created_at = None;
    let mut updated_at = None;
    let mut recognized = false;

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(kind) = record.get("type").and_then(Value::as_str) else {
            continue;
        };
        if cwd.is_none() {
            cwd = record
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(|value| non_empty(value.to_string()));
        }
        if let Some(timestamp) = record
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
        {
            created_at = Some(created_at.map_or(timestamp, |current: i64| current.min(timestamp)));
            updated_at = Some(updated_at.map_or(timestamp, |current: i64| current.max(timestamp)));
        }

        match kind {
            "custom-title" => {
                recognized = true;
                custom_title = string_field(&record, &["customTitle", "title"]);
            }
            "agent-name" => {
                recognized = true;
                agent_name = string_field(&record, &["agentName"]);
            }
            "ai-title" => {
                recognized = true;
                ai_title = string_field(&record, &["aiTitle"]);
            }
            "last-prompt" => {
                recognized = true;
                if let Some(prompt) = string_field(&record, &["lastPrompt"]).and_then(usable_prompt)
                {
                    last_prompt = Some(prompt);
                }
            }
            "user"
                if !record
                    .get("isSidechain")
                    .and_then(Value::as_bool)
                    .unwrap_or(false) =>
            {
                recognized = true;
                if let Some(prompt) = record
                    .pointer("/message/content")
                    .and_then(message_text)
                    .and_then(non_empty)
                    .and_then(usable_prompt)
                {
                    if first_prompt.is_none() {
                        first_prompt = Some(prompt.clone());
                    }
                    last_prompt = Some(prompt);
                }
            }
            "assistant" | "mode" | "permission-mode" => {
                recognized = true;
            }
            _ => {}
        }
    }

    if !recognized {
        return None;
    }
    let name = custom_title.or(agent_name).or(ai_title).map(compact_text);
    let preview = compact_text(last_prompt.or(first_prompt).unwrap_or_default());
    if name.is_none() && preview.is_empty() {
        return None;
    }
    Some(SessionSummary {
        id,
        name,
        preview,
        cwd,
        source: Some("cli".to_string()),
        created_at,
        updated_at: updated_at.unwrap_or(modified_at),
    })
}
