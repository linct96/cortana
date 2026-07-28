use super::{codex::auth_path, *};
use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};
use prost::Message;
use rusqlite::OpenFlags;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{BufRead, BufReader},
};

const UNKNOWN_MODEL: &str = "未知模型";

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum UsageRange {
    Today,
    #[serde(rename = "7d")]
    SevenDays,
    #[serde(rename = "30d")]
    ThirtyDays,
    All,
}

#[derive(Clone, Default, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelUsage {
    model: String,
    tokens: TokenUsage,
    session_count: usize,
    turn_count: usize,
    estimated_cost_usd: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageBucket {
    key: String,
    label: String,
    total_tokens: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UsageAnalytics {
    total: TokenUsage,
    estimated_cost_usd: f64,
    unpriced_model_count: usize,
    session_count: usize,
    turn_count: usize,
    active_days: usize,
    models: Vec<ModelUsage>,
    trend: Vec<UsageBucket>,
    skipped_files: usize,
}

#[derive(Clone)]
struct UsageRecord {
    timestamp: DateTime<Local>,
    session_id: String,
    turn_id: String,
    turn_count: u64,
    model: String,
    tokens: TokenUsage,
}

#[derive(Default)]
struct ParsedUsage {
    totals: Vec<UsageRecord>,
    models: Vec<UsageRecord>,
    skipped_files: usize,
}

#[derive(Default)]
struct ModelAccumulator {
    tokens: TokenUsage,
    sessions: HashSet<String>,
    turns: HashSet<String>,
}

#[derive(Default)]
struct AnalyticsAccumulator {
    total: TokenUsage,
    sessions: HashSet<String>,
    turns: HashSet<String>,
    active_days: HashSet<NaiveDate>,
    models: HashMap<String, ModelAccumulator>,
    trend: BTreeMap<String, u64>,
    skipped_files: usize,
}

#[tauri::command]
pub(super) async fn get_usage_analytics(
    state: State<'_, AppState>,
    product: AccountProduct,
    range: UsageRange,
) -> Result<UsageAnalytics, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || aggregate_usage(&state, product, range))
        .await
        .map_err(|error| error.to_string())?
}

fn aggregate_usage(
    state: &AppState,
    product: AccountProduct,
    range: UsageRange,
) -> Result<UsageAnalytics, String> {
    let today = Local::now().date_naive();
    let start_date = range_start(range, today);
    let parsed = match product {
        AccountProduct::Codex => parse_codex(state, start_date)?,
        AccountProduct::Claude => parse_claude(home_dir(state).join(".claude"), start_date),
        AccountProduct::Antigravity => {
            parse_antigravity(home_dir(state).join(".gemini/antigravity-cli"))
        }
        AccountProduct::Grok => parse_grok(home_dir(state).join(".grok"), start_date),
    };
    finish_analytics(state, parsed, range, today)
}

fn home_dir(state: &AppState) -> PathBuf {
    state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home)
        .to_path_buf()
}

fn finish_analytics(
    state: &AppState,
    parsed: ParsedUsage,
    range: UsageRange,
    today: NaiveDate,
) -> Result<UsageAnalytics, String> {
    let start_date = range_start(range, today);
    let mut analytics = AnalyticsAccumulator {
        skipped_files: parsed.skipped_files,
        ..Default::default()
    };

    for record in parsed.totals {
        if !in_range(record.timestamp.date_naive(), start_date, today) {
            continue;
        }
        analytics.total.add(&record.tokens);
        analytics.sessions.insert(record.session_id.clone());
        analytics.active_days.insert(record.timestamp.date_naive());
        add_turns(
            &mut analytics.turns,
            &record.session_id,
            &record.turn_id,
            record.turn_count,
        );
        *analytics
            .trend
            .entry(bucket_key(range, record.timestamp))
            .or_default() += record.tokens.total_tokens;
    }

    for record in parsed.models {
        if !in_range(record.timestamp.date_naive(), start_date, today) {
            continue;
        }
        let usage = analytics.models.entry(record.model).or_default();
        usage.tokens.add(&record.tokens);
        usage.sessions.insert(record.session_id.clone());
        add_turns(
            &mut usage.turns,
            &record.session_id,
            &record.turn_id,
            record.turn_count,
        );
    }

    let pricing = billing::load_pricing(&db::open_database(state)?)?;
    let mut estimated_cost_usd = 0.0;
    let mut unpriced_model_count = 0;
    let mut models = analytics
        .models
        .into_iter()
        .map(|(model, usage)| {
            let estimated_cost = billing::estimated_cost(
                &pricing,
                &model,
                usage.tokens.input_tokens,
                usage.tokens.cached_input_tokens,
                usage.tokens.cache_write_input_tokens,
                usage.tokens.output_tokens,
            );
            if let Some(cost) = estimated_cost {
                estimated_cost_usd += cost;
            } else {
                unpriced_model_count += 1;
            }
            ModelUsage {
                model,
                tokens: usage.tokens,
                session_count: usage.sessions.len(),
                turn_count: usage.turns.len(),
                estimated_cost_usd: estimated_cost,
            }
        })
        .collect::<Vec<_>>();
    models.sort_by_key(|model| std::cmp::Reverse(model.tokens.total_tokens));

    Ok(UsageAnalytics {
        total: analytics.total,
        estimated_cost_usd,
        unpriced_model_count,
        session_count: analytics.sessions.len(),
        turn_count: analytics.turns.len(),
        active_days: analytics.active_days.len(),
        models,
        trend: build_trend(range, today, analytics.trend),
        skipped_files: analytics.skipped_files,
    })
}

fn add_turns(turns: &mut HashSet<String>, session: &str, turn: &str, count: u64) {
    for index in 0..count {
        turns.insert(format!("{session}:{turn}:{index}"));
    }
}

fn in_range(date: NaiveDate, start: Option<NaiveDate>, today: NaiveDate) -> bool {
    date <= today && start.is_none_or(|start| date >= start)
}

fn parse_codex(state: &AppState, start_date: Option<NaiveDate>) -> Result<ParsedUsage, String> {
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

fn parse_codex_file(path: &Path) -> Result<Vec<UsageRecord>, ()> {
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

fn codex_tokens(value: &Value) -> Option<TokenUsage> {
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

fn parse_claude(default_home: PathBuf, start_date: Option<NaiveDate>) -> ParsedUsage {
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

fn claude_top_session(path: &Path) -> String {
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

fn claude_tokens(value: &Value) -> Option<TokenUsage> {
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

fn parse_grok(default_home: PathBuf, start_date: Option<NaiveDate>) -> ParsedUsage {
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

fn grok_tokens(value: &Value) -> Option<TokenUsage> {
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

fn grok_timestamp(record: &Value) -> Option<DateTime<Local>> {
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

#[derive(Clone, PartialEq, Message)]
struct StoredGenerationMetadata {
    #[prost(message, optional, tag = "1")]
    generation: Option<GenerationMetadata>,
    #[prost(int64, repeated, packed, tag = "2")]
    step_indices: Vec<i64>,
}

#[derive(Clone, PartialEq, Message)]
struct GenerationMetadata {
    #[prost(bytes, optional, tag = "1")]
    created_at: Option<Vec<u8>>,
    #[prost(message, optional, tag = "4")]
    usage: Option<AntigravityTokenUsage>,
    #[prost(bytes, optional, tag = "7")]
    completed_at: Option<Vec<u8>>,
    #[prost(string, tag = "19")]
    model_id: String,
}

#[derive(Clone, PartialEq, Message)]
struct AntigravityTokenUsage {
    #[prost(uint64, tag = "2")]
    input_tokens: u64,
    #[prost(uint64, tag = "3")]
    output_tokens: u64,
    #[prost(uint64, tag = "5")]
    cached_input_tokens: u64,
    #[prost(uint64, tag = "9")]
    reasoning_output_tokens: u64,
    #[prost(string, tag = "11")]
    message_id: String,
}

#[derive(Clone, PartialEq, Message)]
struct StepMetadata {
    #[prost(message, optional, tag = "8")]
    completed_at: Option<ProtoTimestamp>,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoTimestamp {
    #[prost(int64, tag = "1")]
    seconds: i64,
    #[prost(int32, tag = "2")]
    nanos: i32,
}

fn parse_antigravity(home: PathBuf) -> ParsedUsage {
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

fn antigravity_parents(path: &Path) -> (HashMap<String, String>, bool) {
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

fn root_session(session: &str, parents: &HashMap<String, String>) -> String {
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

fn parse_antigravity_db(path: &Path, root: &str) -> Result<(Vec<UsageRecord>, bool), ()> {
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

fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

fn proto_timestamp(timestamp: ProtoTimestamp) -> Option<DateTime<Local>> {
    DateTime::<Utc>::from_timestamp(timestamp.seconds, timestamp.nanos.try_into().ok()?)
        .map(|value| value.with_timezone(&Local))
}

fn decode_proto_timestamp(data: Vec<u8>) -> Option<DateTime<Local>> {
    ProtoTimestamp::decode(data.as_slice())
        .ok()
        .and_then(proto_timestamp)
}

fn read_jsonl(path: &Path) -> Result<Vec<Value>, ()> {
    let file = fs::File::open(path).map_err(|_| ())?;
    let mut values = Vec::new();
    let mut nonempty = false;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|_| ())?;
        if line.trim().is_empty() {
            continue;
        }
        nonempty = true;
        if let Ok(value) = serde_json::from_str(&line) {
            values.push(value);
        }
    }
    if nonempty && values.is_empty() {
        Err(())
    } else {
        Ok(values)
    }
}

fn collect_files(directory: &Path, extension: &str, files: &mut Vec<PathBuf>) -> usize {
    collect_matching_files(directory, files, &|path| {
        path.extension().is_some_and(|value| value == extension)
    })
}

fn collect_named_files(directory: &Path, name: &str, files: &mut Vec<PathBuf>) -> usize {
    collect_matching_files(directory, files, &|path| {
        path.file_name().is_some_and(|value| value == name)
    })
}

fn collect_matching_files(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    matches: &impl Fn(&Path) -> bool,
) -> usize {
    if !directory.exists() {
        return 0;
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return 1,
    };
    let mut skipped = 0;
    for entry in entries {
        let Ok(entry) = entry else {
            skipped += 1;
            continue;
        };
        let path = entry.path();
        if path.is_dir() {
            skipped += collect_matching_files(&path, files, matches);
        } else if matches(&path) {
            files.push(path);
        }
    }
    skipped
}

fn file_predates_range(path: &Path, start_date: Option<NaiveDate>) -> bool {
    let Some(start_date) = start_date else {
        return false;
    };
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(|modified| DateTime::<Local>::from(modified).date_naive() < start_date)
        .unwrap_or(false)
}

fn json_timestamp(value: &Value) -> Option<DateTime<Local>> {
    value
        .as_str()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Local))
}

fn range_start(range: UsageRange, today: NaiveDate) -> Option<NaiveDate> {
    match range {
        UsageRange::Today => Some(today),
        UsageRange::SevenDays => today.checked_sub_days(chrono::Days::new(6)),
        UsageRange::ThirtyDays => today.checked_sub_days(chrono::Days::new(29)),
        UsageRange::All => None,
    }
}

fn bucket_key(range: UsageRange, timestamp: DateTime<Local>) -> String {
    match range {
        UsageRange::Today => timestamp.format("%Y-%m-%dT%H").to_string(),
        UsageRange::SevenDays | UsageRange::ThirtyDays => timestamp.format("%Y-%m-%d").to_string(),
        UsageRange::All => timestamp.format("%Y-%m").to_string(),
    }
}

fn build_trend(
    range: UsageRange,
    today: NaiveDate,
    values: BTreeMap<String, u64>,
) -> Vec<UsageBucket> {
    match range {
        UsageRange::Today => (0..24)
            .map(|hour| {
                let key = format!("{}T{hour:02}", today.format("%Y-%m-%d"));
                bucket(&key, format!("{hour:02}:00"), &values)
            })
            .collect(),
        UsageRange::SevenDays | UsageRange::ThirtyDays => {
            let start = range_start(range, today).unwrap_or(today);
            let days = (today - start).num_days();
            (0..=days)
                .filter_map(|offset| start.checked_add_days(chrono::Days::new(offset as u64)))
                .map(|date| {
                    let key = date.format("%Y-%m-%d").to_string();
                    bucket(&key, date.format("%m-%d").to_string(), &values)
                })
                .collect()
        }
        UsageRange::All => {
            let Some(first) = values.keys().next().and_then(|key| parse_month(key)) else {
                return Vec::new();
            };
            let mut month = first;
            let mut buckets = Vec::new();
            while month <= today {
                let key = month.format("%Y-%m").to_string();
                buckets.push(bucket(&key, key.clone(), &values));
                month = next_month(month);
            }
            buckets
        }
    }
}

fn bucket(key: &str, label: String, values: &BTreeMap<String, u64>) -> UsageBucket {
    UsageBucket {
        key: key.to_string(),
        label,
        total_tokens: values.get(key).copied().unwrap_or_default(),
    }
}

fn parse_month(value: &str) -> Option<NaiveDate> {
    let mut parts = value.split('-');
    NaiveDate::from_ymd_opt(parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, 1)
}

fn next_month(date: NaiveDate) -> NaiveDate {
    let (year, month) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1).expect("valid month")
}

impl TokenUsage {
    fn add(&mut self, other: &Self) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.cache_write_input_tokens += other.cache_write_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.total_tokens += other.total_tokens;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
        db.execute_batch("CREATE TABLE steps (idx INTEGER, metadata BLOB); CREATE TABLE gen_metadata (data BLOB);").unwrap();
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
}
