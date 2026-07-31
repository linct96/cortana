use super::{
    aggregate::{collect_named_files, file_predates_range, read_jsonl},
    types::{ParsedUsage, TokenUsage, UsageRecord, UNKNOWN_MODEL},
};
use chrono::{DateTime, Local, NaiveDate, Utc};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub(super) fn parse_grok(default_home: PathBuf, start_date: Option<NaiveDate>) -> ParsedUsage {
    let home = std::env::var_os("GROK_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or(default_home);
    let mut files = Vec::new();
    let mut parsed = ParsedUsage::default();
    parsed.skipped_files +=
        collect_named_files(&home.join("sessions"), "updates.jsonl", &mut files);
    files.retain(|path| !file_predates_range(path, start_date));
    let mut prompts: HashMap<String, (UsageRecord, Vec<UsageRecord>)> = HashMap::new();

    for path in files {
        let records = match read_jsonl(&path) {
            Ok(records) => records,
            Err(()) => {
                parsed.skipped_files += 1;
                continue;
            }
        };
        for record in records {
            let update = &record["params"]["update"];
            let usage = &update["usage"];
            if !usage.is_object() {
                continue;
            }
            let Some(timestamp) = grok_timestamp(&record) else {
                continue;
            };
            let session_id = record["params"]["sessionId"]
                .as_str()
                .or_else(|| {
                    path.parent()
                        .and_then(Path::file_name)
                        .and_then(|v| v.to_str())
                })
                .unwrap_or_default()
                .to_string();
            let Some(turn_id) = update["prompt_id"]
                .as_str()
                .or_else(|| record["params"]["_meta"]["eventId"].as_str())
            else {
                continue;
            };
            let Some(tokens) = grok_tokens(usage) else {
                continue;
            };
            let total = UsageRecord {
                timestamp,
                session_id: session_id.clone(),
                turn_id: turn_id.to_string(),
                turn_count: usage["numTurns"].as_u64().unwrap_or_default(),
                model: UNKNOWN_MODEL.to_string(),
                tokens,
            };
            let models = usage["modelUsage"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(model, value)| {
                    Some(UsageRecord {
                        timestamp,
                        session_id: session_id.clone(),
                        turn_id: turn_id.to_string(),
                        turn_count: value["modelCalls"].as_u64().unwrap_or_default(),
                        model: model.clone(),
                        tokens: grok_tokens(value)?,
                    })
                })
                .collect::<Vec<_>>();
            let key = format!("{session_id}:{turn_id}");
            match prompts.get(&key) {
                Some((existing, _))
                    if existing.tokens.total_tokens > total.tokens.total_tokens
                        || (existing.tokens.total_tokens == total.tokens.total_tokens
                            && existing.timestamp >= total.timestamp) => {}
                _ => {
                    prompts.insert(key, (total, models));
                }
            }
        }
    }
    for (total, models) in prompts.into_values() {
        parsed.totals.push(total);
        parsed.models.extend(models);
    }
    parsed
}

pub(super) fn grok_tokens(value: &Value) -> Option<TokenUsage> {
    let input = value.get("inputTokens")?.as_u64()?;
    let output = value.get("outputTokens")?.as_u64()?;
    Some(TokenUsage {
        input_tokens: input,
        cached_input_tokens: value
            .get("cachedReadTokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        cache_write_input_tokens: 0,
        output_tokens: output,
        reasoning_output_tokens: value
            .get("reasoningTokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        total_tokens: input + output,
    })
}

pub(super) fn grok_timestamp(record: &Value) -> Option<DateTime<Local>> {
    record["params"]["_meta"]["agentTimestampMs"]
        .as_i64()
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .or_else(|| {
            record["timestamp"]
                .as_i64()
                .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0))
        })
        .map(|timestamp| timestamp.with_timezone(&Local))
}
