use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(super) const UNKNOWN_MODEL: &str = "未知模型";

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum UsageRange {
    Today,
    #[serde(rename = "7d")]
    SevenDays,
    #[serde(rename = "30d")]
    ThirtyDays,
    All,
}

#[derive(Clone, Default, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TokenUsage {
    pub(super) input_tokens: u64,
    pub(super) cached_input_tokens: u64,
    pub(super) cache_write_input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) reasoning_output_tokens: u64,
    pub(super) total_tokens: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ModelUsage {
    pub(super) model: String,
    pub(super) tokens: TokenUsage,
    pub(super) session_count: usize,
    pub(super) turn_count: usize,
    pub(super) estimated_cost_usd: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UsageBucket {
    pub(super) key: String,
    pub(super) label: String,
    pub(super) total_tokens: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageAnalytics {
    pub(super) total: TokenUsage,
    pub(super) estimated_cost_usd: f64,
    pub(super) unpriced_model_count: usize,
    pub(super) session_count: usize,
    pub(super) turn_count: usize,
    pub(super) active_days: usize,
    pub(super) models: Vec<ModelUsage>,
    pub(super) trend: Vec<UsageBucket>,
    pub(super) skipped_files: usize,
}

#[derive(Clone)]
pub(super) struct UsageRecord {
    pub(super) timestamp: DateTime<Local>,
    pub(super) session_id: String,
    pub(super) turn_id: String,
    pub(super) turn_count: u64,
    pub(super) model: String,
    pub(super) tokens: TokenUsage,
}

#[derive(Default)]
pub(super) struct ParsedUsage {
    pub(super) totals: Vec<UsageRecord>,
    pub(super) models: Vec<UsageRecord>,
    pub(super) skipped_files: usize,
}

#[derive(Default)]
pub(super) struct ModelAccumulator {
    pub(super) tokens: TokenUsage,
    pub(super) sessions: HashSet<String>,
    pub(super) turns: HashSet<String>,
}

#[derive(Default)]
pub(super) struct AnalyticsAccumulator {
    pub(super) total: TokenUsage,
    pub(super) sessions: HashSet<String>,
    pub(super) turns: HashSet<String>,
    pub(super) active_days: HashSet<NaiveDate>,
    pub(super) models: HashMap<String, ModelAccumulator>,
    pub(super) trend: BTreeMap<String, u64>,
    pub(super) skipped_files: usize,
}
