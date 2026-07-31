use super::{
    aggregate::{collect_files, file_predates_range, json_timestamp, read_jsonl},
    types::{ParsedUsage, TokenUsage, UsageRecord, UNKNOWN_MODEL},
};
use chrono::NaiveDate;
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub(super) fn parse_claude(default_home: PathBuf, start_date: Option<NaiveDate>) -> ParsedUsage {
    let home = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or(default_home);
    let mut files = Vec::new();
    let mut parsed = ParsedUsage::default();
    parsed.skipped_files += collect_files(&home.join("projects"), "jsonl", &mut files);
    files.retain(|path| !file_predates_range(path, start_date));
    let mut messages: HashMap<(String, String), UsageRecord> = HashMap::new();

    for path in files {
        let top_session = claude_top_session(&path);
        let records = match read_jsonl(&path) {
            Ok(records) => records,
            Err(()) => {
                parsed.skipped_files += 1;
                continue;
            }
        };
        for record in records {
            if record["type"] != "assistant" {
                continue;
            }
            let message = &record["message"];
            let Some(message_id) = message["id"].as_str() else {
                continue;
            };
            let Some(tokens) = claude_tokens(&message["usage"]) else {
                continue;
            };
            let Some(timestamp) = json_timestamp(&record["timestamp"]) else {
                continue;
            };
            let session_id = record["sessionId"]
                .as_str()
                .filter(|_| {
                    !path
                        .components()
                        .any(|part| part.as_os_str() == "subagents")
                })
                .unwrap_or(&top_session)
                .to_string();
            let usage = UsageRecord {
                timestamp,
                session_id: session_id.clone(),
                turn_id: message_id.to_string(),
                turn_count: 1,
                model: message["model"]
                    .as_str()
                    .unwrap_or(UNKNOWN_MODEL)
                    .to_string(),
                tokens,
            };
            let key = (session_id, message_id.to_string());
            match messages.get(&key) {
                Some(existing)
                    if existing.tokens.total_tokens > usage.tokens.total_tokens
                        || (existing.tokens.total_tokens == usage.tokens.total_tokens
                            && existing.timestamp >= usage.timestamp) => {}
                _ => {
                    messages.insert(key, usage);
                }
            }
        }
    }
    parsed.totals = messages.into_values().collect();
    parsed.models = parsed.totals.clone();
    parsed
}

pub(super) fn claude_top_session(path: &Path) -> String {
    let components = path.components().collect::<Vec<_>>();
    components
        .iter()
        .position(|part| part.as_os_str() == "subagents")
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| components.get(index))
        .and_then(|part| Path::new(part.as_os_str()).file_name())
        .and_then(|name| name.to_str())
        .or_else(|| path.file_stem().and_then(|name| name.to_str()))
        .unwrap_or_default()
        .to_string()
}

pub(super) fn claude_tokens(value: &Value) -> Option<TokenUsage> {
    let fresh = value.get("input_tokens")?.as_u64()?;
    let cache_write = value
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let cache_read = value
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let output = value.get("output_tokens")?.as_u64()?;
    let input = fresh + cache_write + cache_read;
    Some(TokenUsage {
        input_tokens: input,
        cached_input_tokens: cache_read,
        cache_write_input_tokens: cache_write,
        output_tokens: output,
        reasoning_output_tokens: 0,
        total_tokens: input + output,
    })
}
