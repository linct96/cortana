use super::{codex::auth_path, *};
use chrono::{Datelike, Local, NaiveDate};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{BufRead, BufReader},
};

const UNKNOWN_MODEL: &str = "未知模型";

#[derive(Clone, Copy, Deserialize)]
pub(super) enum UsageRange {
    #[serde(rename = "today")]
    Today,
    #[serde(rename = "7d")]
    SevenDays,
    #[serde(rename = "30d")]
    ThirtyDays,
    #[serde(rename = "all")]
    All,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
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

#[derive(Default)]
struct ModelAccumulator {
    tokens: TokenUsage,
    sessions: HashSet<PathBuf>,
    turns: HashSet<String>,
}

#[derive(Default)]
struct AnalyticsAccumulator {
    total: TokenUsage,
    sessions: HashSet<PathBuf>,
    turns: HashSet<String>,
    active_days: HashSet<NaiveDate>,
    models: HashMap<String, ModelAccumulator>,
    trend: BTreeMap<String, u64>,
    skipped_files: usize,
}

#[tauri::command]
pub(super) async fn get_codex_usage_analytics(
    state: State<'_, AppState>,
    range: UsageRange,
) -> Result<UsageAnalytics, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || aggregate_usage(&state, range))
        .await
        .map_err(|error| error.to_string())?
}

fn aggregate_usage(state: &AppState, range: UsageRange) -> Result<UsageAnalytics, String> {
    let codex_home = auth_path(state)?
        .parent()
        .ok_or_else(|| "无法定位 Codex 主目录。".to_string())?
        .to_path_buf();
    let today = Local::now().date_naive();
    let start_date = range_start(range, today);
    let mut files = Vec::new();
    for directory in [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ] {
        collect_jsonl_files(&directory, start_date, &mut files)?;
    }

    let mut analytics = AnalyticsAccumulator::default();
    for file in files {
        if parse_session_file(&file, range, start_date, today, &mut analytics).is_err() {
            analytics.skipped_files += 1;
        }
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
    models.sort_by(|left, right| right.tokens.total_tokens.cmp(&left.tokens.total_tokens));

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

fn collect_jsonl_files(
    directory: &Path,
    start_date: Option<NaiveDate>,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, start_date, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
            && !file_predates_range(&path, start_date)
        {
            files.push(path);
        }
    }
    Ok(())
}

fn file_predates_range(path: &Path, start_date: Option<NaiveDate>) -> bool {
    let Some(start_date) = start_date else {
        return false;
    };
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(|modified| chrono::DateTime::<Local>::from(modified).date_naive() < start_date)
        .unwrap_or(false)
}

fn parse_session_file(
    path: &Path,
    range: UsageRange,
    start_date: Option<NaiveDate>,
    today: NaiveDate,
    analytics: &mut AnalyticsAccumulator,
) -> Result<(), String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut model: Option<String> = None;
    let mut turn_id: Option<String> = None;

    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| error.to_string())?;
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match record.get("type").and_then(Value::as_str) {
            Some("turn_context") => {
                model = record["payload"]["model"].as_str().map(str::to_string);
                turn_id = record["payload"]["turn_id"].as_str().map(str::to_string);
            }
            Some("event_msg") if record["payload"]["type"] == "token_count" => {
                let Some(timestamp) = record
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.with_timezone(&Local))
                else {
                    continue;
                };
                let date = timestamp.date_naive();
                if date > today || start_date.is_some_and(|start| date < start) {
                    continue;
                }
                let Some(tokens) = token_usage(&record["payload"]["info"]["last_token_usage"])
                else {
                    continue;
                };
                let model = model.clone().unwrap_or_else(|| UNKNOWN_MODEL.to_string());
                analytics.total.add(&tokens);
                analytics.sessions.insert(path.to_path_buf());
                analytics.active_days.insert(date);
                *analytics
                    .trend
                    .entry(bucket_key(range, timestamp))
                    .or_default() += tokens.total_tokens;

                let model_usage = analytics.models.entry(model).or_default();
                model_usage.tokens.add(&tokens);
                model_usage.sessions.insert(path.to_path_buf());
                if let Some(turn_id) = &turn_id {
                    analytics.turns.insert(turn_id.clone());
                    model_usage.turns.insert(turn_id.clone());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn token_usage(value: &Value) -> Option<TokenUsage> {
    let usage = TokenUsage {
        input_tokens: value.get("input_tokens")?.as_u64()?,
        cached_input_tokens: value.get("cached_input_tokens")?.as_u64()?,
        output_tokens: value.get("output_tokens")?.as_u64()?,
        reasoning_output_tokens: value.get("reasoning_output_tokens")?.as_u64()?,
        total_tokens: value.get("total_tokens")?.as_u64()?,
    };
    (usage.input_tokens > 0 || usage.output_tokens > 0).then_some(usage)
}

impl TokenUsage {
    fn add(&mut self, other: &Self) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.total_tokens += other.total_tokens;
    }
}

fn range_start(range: UsageRange, today: NaiveDate) -> Option<NaiveDate> {
    match range {
        UsageRange::Today => Some(today),
        UsageRange::SevenDays => today.checked_sub_days(chrono::Days::new(6)),
        UsageRange::ThirtyDays => today.checked_sub_days(chrono::Days::new(29)),
        UsageRange::All => None,
    }
}

fn bucket_key(range: UsageRange, timestamp: chrono::DateTime<Local>) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parses_incremental_usage_without_double_counting() {
        let directory = std::env::temp_dir().join(format!("cortana-analytics-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("session.jsonl");
        let today = Local::now().date_naive();
        let timestamp = Local
            .with_ymd_and_hms(today.year(), today.month(), today.day(), 0, 0, 0)
            .single()
            .unwrap()
            .to_rfc3339();
        let content = [
            json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":4729}}}}).to_string(),
            json!({"type":"turn_context","payload":{"model":"model-a","turn_id":"turn-a"}}).to_string(),
            json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":3,"reasoning_output_tokens":1,"total_tokens":13},"total_token_usage":{"total_tokens":13}}}}).to_string(),
            "{incomplete".to_string(),
            json!({"type":"turn_context","payload":{"model":"model-b","turn_id":"turn-b"}}).to_string(),
            json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":20,"cached_input_tokens":8,"output_tokens":5,"reasoning_output_tokens":2,"total_tokens":25},"total_token_usage":{"total_tokens":38}}}}).to_string(),
        ].join("\n");
        fs::write(&path, content).unwrap();

        let mut analytics = AnalyticsAccumulator::default();
        parse_session_file(&path, UsageRange::Today, Some(today), today, &mut analytics).unwrap();

        assert_eq!(analytics.total.total_tokens, 38);
        assert_eq!(analytics.total.cached_input_tokens, 12);
        assert_eq!(analytics.models.len(), 2);
        assert_eq!(analytics.turns.len(), 2);
        assert_eq!(analytics.sessions.len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
