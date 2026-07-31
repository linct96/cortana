use super::{
    aggregate::{collect_files, file_predates_range, json_timestamp, read_jsonl},
    types::{ParsedUsage, TokenUsage, UsageRecord, UNKNOWN_MODEL},
};
use crate::{platform::state::AppState, products::codex::auth_path};
use chrono::NaiveDate;
use serde_json::Value;
use std::path::Path;

pub(super) fn parse_codex(
    state: &AppState,
    start_date: Option<NaiveDate>,
) -> Result<ParsedUsage, String> {
    let codex_home = auth_path(state).and_then(|path| {
        path.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "无法定位 Codex 主目录。".to_string())
    })?;
    let mut files = Vec::new();
    let mut parsed = ParsedUsage::default();
    for directory in [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ] {
        parsed.skipped_files += collect_files(&directory, "jsonl", &mut files);
    }
    files.retain(|path| !file_predates_range(path, start_date));
    for path in files {
        match parse_codex_file(&path) {
            Ok(records) => {
                parsed.totals.extend(records.iter().cloned());
                parsed.models.extend(records);
            }
            Err(()) => parsed.skipped_files += 1,
        }
    }
    Ok(parsed)
}

pub(super) fn parse_codex_file(path: &Path) -> Result<Vec<UsageRecord>, ()> {
    let records = read_jsonl(path)?;
    let mut session_id = path.display().to_string();
    let mut model = UNKNOWN_MODEL.to_string();
    let mut turn_id = String::new();
    let mut usage = Vec::new();
    for record in records {
        match record.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                if let Some(id) = record["payload"]["id"].as_str() {
                    session_id = id.to_string();
                }
            }
            Some("turn_context") => {
                model = record["payload"]["model"]
                    .as_str()
                    .unwrap_or(UNKNOWN_MODEL)
                    .to_string();
                turn_id = record["payload"]["turn_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
            Some("event_msg") if record["payload"]["type"] == "token_count" => {
                let Some(timestamp) = json_timestamp(&record["timestamp"]) else {
                    continue;
                };
                let Some(tokens) = codex_tokens(&record["payload"]["info"]["last_token_usage"])
                else {
                    continue;
                };
                usage.push(UsageRecord {
                    timestamp,
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    turn_count: u64::from(!turn_id.is_empty()),
                    model: model.clone(),
                    tokens,
                });
            }
            _ => {}
        }
    }
    Ok(usage)
}

pub(super) fn codex_tokens(value: &Value) -> Option<TokenUsage> {
    let input = value.get("input_tokens")?.as_u64()?;
    let output = value.get("output_tokens")?.as_u64()?;
    let cached = value.get("cached_input_tokens")?.as_u64()?;
    let reasoning = value.get("reasoning_output_tokens")?.as_u64()?;
    (input > 0 || output > 0).then(|| TokenUsage {
        input_tokens: input,
        cached_input_tokens: cached,
        cache_write_input_tokens: 0,
        output_tokens: output,
        reasoning_output_tokens: reasoning,
        total_tokens: input + output,
    })
}
