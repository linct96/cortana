use super::{
    aggregate::{file_predates_range, json_timestamp, read_jsonl},
    types::{ParsedUsage, TokenUsage, UsageRecord, UNKNOWN_MODEL},
};
use crate::{platform::state::AppState, products::pi::auth::pi_agent_dir};
use chrono::NaiveDate;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) fn parse_pi(
    state: &AppState,
    start_date: Option<NaiveDate>,
) -> Result<ParsedUsage, String> {
    let sessions = pi_agent_dir(state).join("sessions");
    let (mut files, skipped_files) = collect_pi_session_files(&sessions);
    let mut parsed = ParsedUsage {
        skipped_files,
        ..Default::default()
    };
    files.retain(|path| !file_predates_range(path, start_date));

    for path in files {
        match parse_pi_file(&path) {
            Ok(records) => {
                parsed.totals.extend(records.iter().cloned());
                parsed.models.extend(records);
            }
            Err(()) => parsed.skipped_files += 1,
        }
    }
    Ok(parsed)
}

pub(super) fn collect_pi_session_files(directory: &Path) -> (Vec<PathBuf>, usize) {
    let projects = match fs::read_dir(directory) {
        Ok(projects) => projects,
        Err(_) if !directory.exists() => return (Vec::new(), 0),
        Err(_) => return (Vec::new(), 1),
    };
    let mut files = Vec::new();
    let mut skipped = 0;
    for project in projects {
        let Ok(project) = project else {
            skipped += 1;
            continue;
        };
        let Ok(entries) = fs::read_dir(project.path()) else {
            if project.path().is_dir() {
                skipped += 1;
            }
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                skipped += 1;
                continue;
            };
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|value| value == "jsonl") {
                files.push(path);
            }
        }
    }
    (files, skipped)
}

pub(super) fn parse_pi_file(path: &Path) -> Result<Vec<UsageRecord>, ()> {
    let entries = read_jsonl(path)?;
    let mut session_id = path.display().to_string();
    let mut records = Vec::new();

    for (index, entry) in entries.into_iter().enumerate() {
        if entry["type"] == "session" {
            if let Some(id) = entry["id"].as_str() {
                session_id = id.to_string();
            }
            continue;
        }

        let (usage, model) = match entry["type"].as_str() {
            Some("message") if entry["message"]["role"] == "assistant" => (
                &entry["message"]["usage"],
                entry["message"]["model"].as_str().unwrap_or(UNKNOWN_MODEL),
            ),
            Some("message") if entry["message"]["role"] == "toolResult" => {
                (&entry["message"]["usage"], UNKNOWN_MODEL)
            }
            Some("compaction" | "branch_summary") => (&entry["usage"], UNKNOWN_MODEL),
            _ => continue,
        };
        let Some(tokens) = pi_tokens(usage) else {
            continue;
        };
        let Some(timestamp) = json_timestamp(&entry["timestamp"]) else {
            continue;
        };
        records.push(UsageRecord {
            timestamp,
            session_id: session_id.clone(),
            turn_id: entry["id"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| format!("{}:{index}", path.display())),
            turn_count: 1,
            model: model.to_string(),
            tokens,
            reported_cost_usd: pi_cost(usage),
        });
    }
    Ok(records)
}

pub(super) fn pi_tokens(value: &Value) -> Option<TokenUsage> {
    let fresh = value.get("input")?.as_u64()?;
    let output = value.get("output")?.as_u64()?;
    let cache_read = value.get("cacheRead")?.as_u64()?;
    let cache_write = value.get("cacheWrite")?.as_u64()?;
    let total = value.get("totalTokens")?.as_u64()?;
    (total > 0).then(|| TokenUsage {
        input_tokens: fresh.saturating_add(cache_read).saturating_add(cache_write),
        cached_input_tokens: cache_read,
        cache_write_input_tokens: cache_write,
        output_tokens: output,
        reasoning_output_tokens: value
            .get("reasoning")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        total_tokens: total,
    })
}

fn pi_cost(value: &Value) -> Option<f64> {
    value["cost"]["total"]
        .as_f64()
        .filter(|cost| cost.is_finite() && *cost >= 0.0)
}
