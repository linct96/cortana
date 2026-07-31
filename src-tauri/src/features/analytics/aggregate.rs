use super::{
    antigravity::parse_antigravity,
    claude::parse_claude,
    codex::parse_codex,
    grok::parse_grok,
    types::{
        AnalyticsAccumulator, ModelUsage, ParsedUsage, TokenUsage, UsageAnalytics, UsageBucket,
        UsageRange,
    },
};
use crate::{
    features::billing,
    platform::{
        db::open_database,
        state::{AccountProduct, AppState},
    },
};
use chrono::{DateTime, Datelike, Local, NaiveDate};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};
use tauri::State;

pub(crate) async fn get_usage_analytics(
    state: State<'_, AppState>,
    product: AccountProduct,
    range: UsageRange,
) -> Result<UsageAnalytics, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || aggregate_usage(&state, product, range))
        .await
        .map_err(|error| error.to_string())?
}

pub(super) fn aggregate_usage(
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

pub(super) fn home_dir(state: &AppState) -> PathBuf {
    state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home)
        .to_path_buf()
}

pub(super) fn finish_analytics(
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

    let pricing = billing::load_pricing(&open_database(state)?)?;
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

pub(super) fn add_turns(turns: &mut HashSet<String>, session: &str, turn: &str, count: u64) {
    for index in 0..count {
        turns.insert(format!("{session}:{turn}:{index}"));
    }
}

pub(super) fn in_range(date: NaiveDate, start: Option<NaiveDate>, today: NaiveDate) -> bool {
    date <= today && start.is_none_or(|start| date >= start)
}

pub(super) fn read_jsonl(path: &Path) -> Result<Vec<Value>, ()> {
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

pub(super) fn collect_files(directory: &Path, extension: &str, files: &mut Vec<PathBuf>) -> usize {
    collect_matching_files(directory, files, &|path| {
        path.extension().is_some_and(|value| value == extension)
    })
}

pub(super) fn collect_named_files(directory: &Path, name: &str, files: &mut Vec<PathBuf>) -> usize {
    collect_matching_files(directory, files, &|path| {
        path.file_name().is_some_and(|value| value == name)
    })
}

pub(super) fn collect_matching_files(
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

pub(super) fn file_predates_range(path: &Path, start_date: Option<NaiveDate>) -> bool {
    let Some(start_date) = start_date else {
        return false;
    };
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(|modified| DateTime::<Local>::from(modified).date_naive() < start_date)
        .unwrap_or(false)
}

pub(super) fn json_timestamp(value: &Value) -> Option<DateTime<Local>> {
    value
        .as_str()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Local))
}

pub(super) fn range_start(range: UsageRange, today: NaiveDate) -> Option<NaiveDate> {
    match range {
        UsageRange::Today => Some(today),
        UsageRange::SevenDays => today.checked_sub_days(chrono::Days::new(6)),
        UsageRange::ThirtyDays => today.checked_sub_days(chrono::Days::new(29)),
        UsageRange::All => None,
    }
}

pub(super) fn bucket_key(range: UsageRange, timestamp: DateTime<Local>) -> String {
    match range {
        UsageRange::Today => timestamp.format("%Y-%m-%dT%H").to_string(),
        UsageRange::SevenDays | UsageRange::ThirtyDays => timestamp.format("%Y-%m-%d").to_string(),
        UsageRange::All => timestamp.format("%Y-%m").to_string(),
    }
}

pub(super) fn build_trend(
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

pub(super) fn bucket(key: &str, label: String, values: &BTreeMap<String, u64>) -> UsageBucket {
    UsageBucket {
        key: key.to_string(),
        label,
        total_tokens: values.get(key).copied().unwrap_or_default(),
    }
}

pub(super) fn parse_month(value: &str) -> Option<NaiveDate> {
    let mut parts = value.split('-');
    NaiveDate::from_ymd_opt(parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, 1)
}

pub(super) fn next_month(date: NaiveDate) -> NaiveDate {
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
