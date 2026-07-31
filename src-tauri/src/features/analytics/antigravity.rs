use super::types::{ParsedUsage, TokenUsage, UsageRecord, UNKNOWN_MODEL};
use chrono::{DateTime, Local, Utc};
use prost::Message;
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, PartialEq, Message)]
pub(super) struct StoredGenerationMetadata {
    #[prost(message, optional, tag = "1")]
    pub(super) generation: Option<GenerationMetadata>,
    #[prost(int64, repeated, packed, tag = "2")]
    pub(super) step_indices: Vec<i64>,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct GenerationMetadata {
    #[prost(bytes, optional, tag = "1")]
    pub(super) created_at: Option<Vec<u8>>,
    #[prost(message, optional, tag = "4")]
    pub(super) usage: Option<AntigravityTokenUsage>,
    #[prost(bytes, optional, tag = "7")]
    pub(super) completed_at: Option<Vec<u8>>,
    #[prost(string, tag = "19")]
    pub(super) model_id: String,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct AntigravityTokenUsage {
    #[prost(uint64, tag = "2")]
    pub(super) input_tokens: u64,
    #[prost(uint64, tag = "3")]
    pub(super) output_tokens: u64,
    #[prost(uint64, tag = "5")]
    pub(super) cached_input_tokens: u64,
    #[prost(uint64, tag = "9")]
    pub(super) reasoning_output_tokens: u64,
    #[prost(string, tag = "11")]
    pub(super) message_id: String,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct StepMetadata {
    #[prost(message, optional, tag = "8")]
    pub(super) completed_at: Option<ProtoTimestamp>,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct ProtoTimestamp {
    #[prost(int64, tag = "1")]
    pub(super) seconds: i64,
    #[prost(int32, tag = "2")]
    pub(super) nanos: i32,
}

pub(super) fn parse_antigravity(home: PathBuf) -> ParsedUsage {
    if !home.exists() {
        return ParsedUsage::default();
    }
    let summaries = home.join("conversation_summaries.db");
    let (parents, summary_failed) = antigravity_parents(&summaries);
    let mut parsed = ParsedUsage {
        skipped_files: usize::from(summary_failed),
        ..Default::default()
    };
    let directory = home.join("conversations");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(_) if !directory.exists() => return parsed,
        Err(_) => {
            parsed.skipped_files += 1;
            return parsed;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            parsed.skipped_files += 1;
            continue;
        };
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "db") {
            continue;
        }
        let session = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let root = root_session(session, &parents);
        match parse_antigravity_db(&path, &root) {
            Ok((records, damaged)) => {
                parsed.skipped_files += usize::from(damaged);
                parsed.totals.extend(records.iter().cloned());
                parsed.models.extend(records);
            }
            Err(()) => parsed.skipped_files += 1,
        }
    }
    parsed
}

pub(super) fn antigravity_parents(path: &Path) -> (HashMap<String, String>, bool) {
    if !path.exists() {
        return (HashMap::new(), false);
    }
    let Ok(connection) = open_read_only(path) else {
        return (HashMap::new(), true);
    };
    let Ok(mut statement) = connection
        .prepare("SELECT conversation_id, parent_conversation_id FROM conversation_summaries")
    else {
        return (HashMap::new(), true);
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return (HashMap::new(), true);
    };
    let mut parents = HashMap::new();
    for row in rows {
        let Ok((child, parent)) = row else {
            return (HashMap::new(), true);
        };
        if !parent.is_empty() {
            parents.insert(child, parent);
        }
    }
    (parents, false)
}

pub(super) fn root_session(session: &str, parents: &HashMap<String, String>) -> String {
    let mut current = session;
    let mut seen = HashSet::new();
    while seen.insert(current) {
        let Some(parent) = parents.get(current) else {
            break;
        };
        current = parent;
    }
    current.to_string()
}

pub(super) fn parse_antigravity_db(
    path: &Path,
    root: &str,
) -> Result<(Vec<UsageRecord>, bool), ()> {
    let connection = open_read_only(path).map_err(|_| ())?;
    let mut step_times = HashMap::new();
    let mut damaged = false;
    let mut steps = connection
        .prepare("SELECT idx, metadata FROM steps WHERE metadata IS NOT NULL")
        .map_err(|_| ())?;
    let rows = steps
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|_| ())?;
    for row in rows {
        let (index, data) = row.map_err(|_| ())?;
        match StepMetadata::decode(data.as_slice()) {
            Ok(metadata) => {
                if let Some(timestamp) = metadata.completed_at.and_then(proto_timestamp) {
                    step_times.insert(index, timestamp);
                }
            }
            Err(_) => damaged = true,
        }
    }
    drop(steps);

    let mut generations = connection
        .prepare("SELECT data FROM gen_metadata")
        .map_err(|_| ())?;
    let rows = generations
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .map_err(|_| ())?;
    let mut records = HashMap::<String, UsageRecord>::new();
    for row in rows {
        let data = row.map_err(|_| ())?;
        let stored = match StoredGenerationMetadata::decode(data.as_slice()) {
            Ok(stored) => stored,
            Err(_) => {
                damaged = true;
                continue;
            }
        };
        let Some(generation) = stored.generation else {
            continue;
        };
        let Some(usage) = generation.usage else {
            continue;
        };
        let Some(timestamp) = stored
            .step_indices
            .iter()
            .rev()
            .find_map(|index| step_times.get(index))
            .copied()
            .or_else(|| generation.completed_at.and_then(decode_proto_timestamp))
            .or_else(|| generation.created_at.and_then(decode_proto_timestamp))
        else {
            continue;
        };
        let input = usage.input_tokens + usage.cached_input_tokens;
        let tokens = TokenUsage {
            input_tokens: input,
            cached_input_tokens: usage.cached_input_tokens,
            cache_write_input_tokens: 0,
            output_tokens: usage.output_tokens,
            reasoning_output_tokens: usage.reasoning_output_tokens,
            total_tokens: input + usage.output_tokens,
        };
        let turn_id = if usage.message_id.is_empty() {
            format!("steps-{:?}", stored.step_indices)
        } else {
            usage.message_id
        };
        let record = UsageRecord {
            timestamp,
            session_id: root.to_string(),
            turn_id: turn_id.clone(),
            turn_count: 1,
            model: if generation.model_id.is_empty() {
                UNKNOWN_MODEL.to_string()
            } else {
                generation.model_id
            },
            tokens,
        };
        match records.get(&turn_id) {
            Some(existing)
                if existing.tokens.total_tokens > record.tokens.total_tokens
                    || (existing.tokens.total_tokens == record.tokens.total_tokens
                        && existing.timestamp >= record.timestamp) => {}
            _ => {
                records.insert(turn_id, record);
            }
        }
    }
    Ok((records.into_values().collect(), damaged))
}

pub(super) fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

pub(super) fn proto_timestamp(timestamp: ProtoTimestamp) -> Option<DateTime<Local>> {
    DateTime::<Utc>::from_timestamp(timestamp.seconds, timestamp.nanos.try_into().ok()?)
        .map(|value| value.with_timezone(&Local))
}

pub(super) fn decode_proto_timestamp(data: Vec<u8>) -> Option<DateTime<Local>> {
    ProtoTimestamp::decode(data.as_slice())
        .ok()
        .and_then(proto_timestamp)
}
