use super::{
    aggregate::finish_analytics,
    antigravity::{
        parse_antigravity, AntigravityTokenUsage, GenerationMetadata, ProtoTimestamp, StepMetadata,
        StoredGenerationMetadata,
    },
    claude::parse_claude,
    codex::parse_codex_file,
    grok::parse_grok,
    pi::{collect_pi_session_files, parse_pi_file},
    types::{ParsedUsage, TokenUsage, UsageRange, UNKNOWN_MODEL},
};
use crate::platform::{db::initialize_database, state::AppState};
use chrono::{DateTime, Datelike, Local, TimeZone};
use prost::Message;
use rusqlite::{params, Connection};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

fn temp_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!("cortana-analytics-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn timestamp() -> DateTime<Local> {
    let today = Local::now().date_naive();
    Local
        .with_ymd_and_hms(today.year(), today.month(), today.day(), 1, 0, 0)
        .single()
        .unwrap()
}

#[test]
fn pi_session_scan_ignores_nested_artifacts() {
    let directory = temp_dir();
    let project = directory.join("project");
    fs::create_dir_all(project.join("artifacts")).unwrap();
    fs::write(project.join("session.jsonl"), "").unwrap();
    fs::write(project.join("artifacts/transcript.jsonl"), "").unwrap();

    let (files, skipped) = collect_pi_session_files(&directory);
    assert_eq!(files, vec![project.join("session.jsonl")]);
    assert_eq!(skipped, 0);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn parses_pi_usage_and_reported_cost() {
    let directory = temp_dir();
    let path = directory.join("session.jsonl");
    let usage = |input, output, cache_read, cache_write, total, reasoning, cost| {
        json!({
            "input": input,
            "output": output,
            "cacheRead": cache_read,
            "cacheWrite": cache_write,
            "reasoning": reasoning,
            "totalTokens": total,
            "cost": { "total": cost }
        })
    };
    let content = [
        json!({"type":"session","version":3,"id":"session-a","timestamp":timestamp().to_rfc3339(),"cwd":"/tmp"}),
        json!({"type":"message","id":"assistant-a","timestamp":timestamp().to_rfc3339(),"message":{"role":"assistant","model":"model-a","usage":usage(10, 4, 3, 2, 19, 1, 0.1)}}),
        json!({"type":"message","id":"tool-a","timestamp":timestamp().to_rfc3339(),"message":{"role":"toolResult","usage":usage(1, 4, 2, 3, 10, 0, 0.2)}}),
        json!({"type":"compaction","id":"compact-a","timestamp":timestamp().to_rfc3339(),"usage":usage(5, 1, 0, 0, 6, 0, 0.3)}),
        json!({"type":"branch_summary","id":"empty-a","timestamp":timestamp().to_rfc3339(),"usage":usage(0, 0, 0, 0, 0, 0, 0.0)}),
    ]
    .map(|value| value.to_string())
    .join("\n");
    fs::write(&path, content).unwrap();

    let records = parse_pi_file(&path).unwrap();
    assert_eq!(records.len(), 3);
    assert!(records
        .iter()
        .all(|record| record.session_id == "session-a"));
    assert_eq!(records[0].model, "model-a");
    assert_eq!(records[1].model, UNKNOWN_MODEL);
    assert_eq!(records[0].tokens.input_tokens, 15);
    assert_eq!(records[0].tokens.reasoning_output_tokens, 1);
    assert_eq!(
        records
            .iter()
            .map(|record| record.tokens.total_tokens)
            .sum::<u64>(),
        35
    );
    assert!(
        (records
            .iter()
            .filter_map(|record| record.reported_cost_usd)
            .sum::<f64>()
            - 0.6)
            .abs()
            < f64::EPSILON
    );

    let state = AppState {
        database_path: directory.join("app.sqlite3"),
        default_codex_home: directory.join(".codex"),
        pending_oauth: Arc::new(Mutex::new(None)),
    };
    initialize_database(&state).unwrap();
    let analytics = finish_analytics(
        &state,
        ParsedUsage {
            totals: records.clone(),
            models: records,
            skipped_files: 0,
        },
        UsageRange::Today,
        timestamp().date_naive(),
    )
    .unwrap();
    assert!((analytics.estimated_cost_usd - 0.6).abs() < f64::EPSILON);
    assert_eq!(analytics.unpriced_model_count, 0);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn parses_codex_last_usage() {
    let directory = temp_dir();
    let path = directory.join("session.jsonl");
    let content = [
            json!({"type":"session_meta","payload":{"id":"session-a"}}).to_string(),
            json!({"type":"turn_context","payload":{"model":"model-a","turn_id":"turn-a"}}).to_string(),
            json!({"timestamp":timestamp().to_rfc3339(),"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":3,"reasoning_output_tokens":1,"total_tokens":999},"total_token_usage":{"total_tokens":9999}}}}).to_string(),
        ]
        .join("\n");
    fs::write(&path, content).unwrap();
    let records = parse_codex_file(&path).unwrap();
    assert_eq!(records[0].tokens.total_tokens, 13);
    assert_eq!(records[0].tokens.cache_write_input_tokens, 0);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn deduplicates_claude_messages_and_counts_cache() {
    let directory = temp_dir();
    let projects = directory.join("projects/project/session-a/subagents");
    fs::create_dir_all(&projects).unwrap();
    let path = projects.join("agent-a.jsonl");
    let record = |input, timestamp: DateTime<Local>| {
        json!({"type":"assistant","timestamp":timestamp.to_rfc3339(),"sessionId":"agent-a","message":{"id":"message-a","model":"claude-test","usage":{"input_tokens":input,"cache_creation_input_tokens":3,"cache_read_input_tokens":4,"output_tokens":5}}}).to_string()
    };
    fs::write(
        path,
        [record(1, timestamp()), record(2, timestamp())].join("\n"),
    )
    .unwrap();
    let parsed = parse_claude(directory.clone(), None);
    assert_eq!(parsed.totals.len(), 1);
    assert_eq!(
        parsed.totals[0].tokens,
        TokenUsage {
            input_tokens: 9,
            cached_input_tokens: 4,
            cache_write_input_tokens: 3,
            output_tokens: 5,
            reasoning_output_tokens: 0,
            total_tokens: 14,
        }
    );
    assert_eq!(parsed.totals[0].session_id, "session-a");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn grok_uses_top_total_and_model_details_once() {
    let directory = temp_dir();
    let sessions = directory.join("sessions/cwd/session-a");
    fs::create_dir_all(&sessions).unwrap();
    let usage = json!({"inputTokens":10,"outputTokens":5,"cachedReadTokens":4,"reasoningTokens":2,"numTurns":1,"modelUsage":{"grok-a":{"inputTokens":7,"outputTokens":3,"cachedReadTokens":2,"reasoningTokens":1,"modelCalls":1},"grok-b":{"inputTokens":3,"outputTokens":2,"cachedReadTokens":2,"reasoningTokens":1,"modelCalls":1}}});
    let record = json!({"timestamp":timestamp().timestamp(),"params":{"sessionId":"session-a","update":{"prompt_id":"prompt-a","usage":usage},"_meta":{"eventId":"event-a"}}}).to_string();
    fs::write(
        sessions.join("updates.jsonl"),
        format!("{record}\n{record}"),
    )
    .unwrap();
    let parsed = parse_grok(directory.clone(), None);
    assert_eq!(parsed.totals.len(), 1);
    assert_eq!(parsed.totals[0].tokens.total_tokens, 15);
    assert_eq!(
        parsed
            .models
            .iter()
            .map(|record| record.tokens.total_tokens)
            .sum::<u64>(),
        15
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn parses_antigravity_parent_session_and_protobuf() {
    let directory = temp_dir();
    let conversations = directory.join("conversations");
    fs::create_dir_all(&conversations).unwrap();
    let summary = Connection::open(directory.join("conversation_summaries.db")).unwrap();
    summary.execute_batch("CREATE TABLE conversation_summaries (conversation_id TEXT, parent_conversation_id TEXT); INSERT INTO conversation_summaries VALUES ('child', 'parent');").unwrap();
    let db = Connection::open(conversations.join("child.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE steps (idx INTEGER, metadata BLOB); CREATE TABLE gen_metadata (data BLOB);",
    )
    .unwrap();
    let step = StepMetadata {
        completed_at: Some(ProtoTimestamp {
            seconds: timestamp().timestamp(),
            nanos: 0,
        }),
    }
    .encode_to_vec();
    db.execute("INSERT INTO steps VALUES (7, ?1)", params![step])
        .unwrap();
    let generation = StoredGenerationMetadata {
        step_indices: vec![7],
        generation: Some(GenerationMetadata {
            created_at: None,
            completed_at: None,
            model_id: "gemini-test".to_string(),
            usage: Some(AntigravityTokenUsage {
                input_tokens: 10,
                output_tokens: 3,
                cached_input_tokens: 4,
                reasoning_output_tokens: 2,
                message_id: "message-a".to_string(),
            }),
        }),
    }
    .encode_to_vec();
    db.execute("INSERT INTO gen_metadata VALUES (?1)", params![generation])
        .unwrap();
    drop(db);
    drop(summary);

    let parsed = parse_antigravity(directory.clone());
    assert_eq!(parsed.skipped_files, 0);
    assert_eq!(parsed.totals[0].session_id, "parent");
    assert_eq!(parsed.totals[0].tokens.total_tokens, 17);
    fs::write(conversations.join("broken.db"), "broken").unwrap();
    assert_eq!(parse_antigravity(directory.clone()).skipped_files, 1);
    fs::remove_dir_all(directory).unwrap();
}
