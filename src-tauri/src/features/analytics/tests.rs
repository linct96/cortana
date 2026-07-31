use super::{
    antigravity::{
        parse_antigravity, AntigravityTokenUsage, GenerationMetadata, ProtoTimestamp, StepMetadata,
        StoredGenerationMetadata,
    },
    claude::parse_claude,
    codex::parse_codex_file,
    grok::parse_grok,
    types::TokenUsage,
};
use chrono::{DateTime, Datelike, Local, TimeZone};
use prost::Message;
use rusqlite::{params, Connection};
use serde_json::json;
use std::{fs, path::PathBuf};
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
