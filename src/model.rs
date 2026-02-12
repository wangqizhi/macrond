use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobConfig {
    pub id: String,
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub schedule: ScheduleConfig,
    pub command: CommandConfig,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ScheduleConfig {
    Cron { expression: String },
    Simple {
        repeat: Repeat,
        time: Option<String>,
        weekday: Option<u8>,
        day: Option<u8>,
        once_at: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    Daily,
    Weekly,
    Monthly,
    EveryMinute,
    Once,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandConfig {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub run_id: String,
    pub job_id: String,
    pub trigger: String,
    pub started_at: DateTime<Local>,
    pub ended_at: DateTime<Local>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub schedule: String,
    pub next_run: Option<DateTime<Local>>,
    pub last_result: Option<ExecutionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    pub updated_at: DateTime<Local>,
    pub pid: u32,
    pub running: bool,
    pub last_reload_error: Option<String>,
    pub jobs: Vec<JobView>,
    pub recent_runs: Vec<ExecutionRecord>,
}

fn default_enabled() -> bool {
    true
}

fn default_timeout() -> u64 {
    3600
}
