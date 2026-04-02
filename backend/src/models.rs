use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

// ── Raw JSONL structures ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct RawEvent {
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub timestamp: Option<String>,
    #[serde(rename = "toolUseResult")]
    pub tool_use_result: Option<Value>,
    pub message: Option<RawMessage>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RawMessage {
    pub model: Option<String>,
    pub id: Option<String>,
    pub content: Option<Vec<Value>>,
    pub usage: Option<RawUsage>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RawUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
}

// ── Domain model ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct UsageRecord {
    pub request_id: String,
    pub session_id: String,
    pub project: String,
    /// "claude-code" or "copilot-proxy"
    pub source: String,
    #[serde(skip_serializing)]
    pub workspace_root: String,
    #[serde(skip_serializing)]
    pub touched_paths: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub subprojects: Vec<String>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_input: f64,
    pub cost_output: f64,
    pub cost_cache_write: f64,
    pub cost_cache_read: f64,
    pub total_cost: f64,
    pub timestamp: DateTime<Utc>,
}

// ── Proxy log structures ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct ProxyLogEntry {
    pub request_id: String,
    pub model: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub timestamp: String,
    pub source: Option<String>,
}

// ── API response types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Default)]
pub struct OverviewResponse {
    pub today: CostSummary,
    pub week: CostSummary,
    pub month: CostSummary,
    pub projected: CostSummary,
    pub week_start_label: String,
    pub month_start_label: String,
    pub daily_spend: Vec<DailySpend>,
    pub hourly_spend: Vec<f64>,
    pub hourly_labels: Vec<String>,
    pub model_series: Vec<ModelSeries>,
    pub cost_breakdown: CostBreakdown,
    pub model_breakdown: Vec<ModelBreakdown>,
    pub activity_heatmap: Vec<HeatmapCell>,
    pub recent_sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CostSummary {
    pub cost: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DailySpend {
    pub date: String,
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ModelSeries {
    pub model: String,
    pub daily: Vec<f64>,   // 14 values aligned with daily_spend dates
    pub hourly: Vec<f64>,  // 24 values, index = hour of day
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CostBreakdown {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ModelBreakdown {
    pub model: String,
    pub cost: f64,
    pub sessions: usize,
    pub pct_of_total: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeatmapCell {
    pub date: String,
    pub cost: f64,
    pub projects: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SessionSummary {
    pub id: String,
    pub project: String,
    pub model: String,
    pub last_active: String,
    pub total_tokens: u64,
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ProjectSummary {
    pub name: String,
    pub total_cost: f64,
    pub sessions: usize,
    pub models: Vec<String>,
    pub subprojects: Vec<SubprojectSummary>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SubprojectSummary {
    pub name: String,
    pub total_cost: f64,
    pub sessions: usize,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RateEntry {
    pub model: String,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_write_per_mtok: f64,
    pub cache_read_per_mtok: f64,
}

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct AppState {
    pub records: Vec<UsageRecord>,
}

pub type SharedState = Arc<RwLock<AppState>>;
pub type BroadcastTx = Arc<broadcast::Sender<String>>;
