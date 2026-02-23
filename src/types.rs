use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{ErrorCode, TimerError, TimerResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobType {
  Once,
  Interval,
  Cron,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MisfirePolicy {
  Skip,
  FireOnce,
  CatchUp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
  Scheduled,
  Running,
  Paused,
  Completed,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
  Success,
  Failed,
  Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
  pub max_retries: u32,
  pub backoff_ms: u64,
}

impl Default for RetryPolicy {
  fn default() -> Self {
    Self {
      max_retries: 0,
      backoff_ms: 1_000,
    }
  }
}

fn default_strict_interval_cycle() -> bool {
  true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobPolicy {
  pub misfire: MisfirePolicy,
  pub max_catchup: u32,
  pub max_runtime_ms: u64,
  pub concurrency: u32,
  #[serde(default = "default_strict_interval_cycle")]
  pub strict_interval_cycle: bool,
  pub max_runs: Option<u32>,
  pub retry: RetryPolicy,
}

impl Default for JobPolicy {
  fn default() -> Self {
    Self {
      misfire: MisfirePolicy::FireOnce,
      max_catchup: 1,
      max_runtime_ms: 30_000,
      concurrency: 1,
      strict_interval_cycle: true,
      max_runs: None,
      retry: RetryPolicy::default(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
  pub run_at: Option<DateTime<Utc>>,
  pub every_ms: Option<u64>,
  pub cron_expr: Option<String>,
  pub tz: Option<String>,
}

impl Schedule {
  pub fn once(run_at: DateTime<Utc>) -> Self {
    Self {
      run_at: Some(run_at),
      every_ms: None,
      cron_expr: None,
      tz: None,
    }
  }

  pub fn interval(every_ms: u64) -> Self {
    Self {
      run_at: None,
      every_ms: Some(every_ms),
      cron_expr: None,
      tz: None,
    }
  }

  pub fn cron(expr: impl Into<String>, tz: impl Into<String>) -> Self {
    Self {
      run_at: None,
      every_ms: None,
      cron_expr: Some(expr.into()),
      tz: Some(tz.into()),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
  pub id: String,
  #[serde(rename = "type")]
  pub job_type: JobType,
  pub schedule: Schedule,
  pub policy: JobPolicy,
  pub payload: Value,
  pub enabled: bool,
  pub deprecated: bool,
}

impl JobSpec {
  pub fn new(job_type: JobType, schedule: Schedule, payload: Value) -> Self {
    Self {
      id: Uuid::new_v4().to_string(),
      job_type,
      schedule,
      policy: JobPolicy::default(),
      payload,
      enabled: true,
      deprecated: false,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
  pub job_id: String,
  pub state: JobState,
  pub last_fired: Option<DateTime<Utc>>,
  pub next_due: Option<DateTime<Utc>>,
  pub failure_count: u32,
  pub run_count: u32,
  pub version: u32,
}

impl RuntimeState {
  pub fn new(job_id: String, next_due: Option<DateTime<Utc>>) -> Self {
    Self {
      job_id,
      state: JobState::Scheduled,
      last_fired: None,
      next_due,
      failure_count: 0,
      run_count: 0,
      version: 1,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
  pub id: String,
  pub job_id: String,
  pub planned_time: DateTime<Utc>,
  pub actual_time: DateTime<Utc>,
  pub status: ExecutionStatus,
  pub error: Option<String>,
}

impl ExecutionRecord {
  pub fn success(job_id: &str, planned_time: DateTime<Utc>, actual_time: DateTime<Utc>) -> Self {
    Self {
      id: Uuid::new_v4().to_string(),
      job_id: job_id.to_string(),
      planned_time,
      actual_time,
      status: ExecutionStatus::Success,
      error: None,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
  pub timestamp: DateTime<Utc>,
  pub event: String,
  pub detail: HashMap<String, String>,
}

impl AuditRecord {
  pub fn new(event: impl Into<String>) -> Self {
    Self {
      timestamp: Utc::now(),
      event: event.into(),
      detail: HashMap::new(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StoreKind {
  Memory,
  Sqlite,
}

impl FromStr for StoreKind {
  type Err = TimerError;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    match s.trim().to_ascii_lowercase().as_str() {
      "memory" => Ok(Self::Memory),
      "sqlite" => Ok(Self::Sqlite),
      _ => Err(TimerError::new(
        ErrorCode::InvalidArgument,
        format!("unknown store kind: {s}"),
      )),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerConfig {
  pub tick_ms: u64,
  pub max_jobs: usize,
  pub payload_max_bytes: usize,
  pub max_global_concurrency: u32,
  pub max_execution_records: usize,
  pub watchdog_timeout_ms: u64,
  pub wall_clock_jump_threshold_ms: i64,
  pub store_kind: StoreKind,
  pub sqlite_path: Option<String>,
  pub restore_on_start: bool,
  pub schema_version: u32,
}

impl Default for TimerConfig {
  fn default() -> Self {
    Self {
      tick_ms: 200,
      max_jobs: 100_000,
      payload_max_bytes: 128 * 1024,
      max_global_concurrency: 1024,
      max_execution_records: 100_000,
      watchdog_timeout_ms: 30_000,
      wall_clock_jump_threshold_ms: 2_000,
      store_kind: StoreKind::Memory,
      sqlite_path: None,
      restore_on_start: false,
      schema_version: 1,
    }
  }
}

pub fn parse_payload(payload_json: Option<&str>) -> TimerResult<Value> {
  let Some(raw) = payload_json else {
    return Ok(Value::Null);
  };
  serde_json::from_str::<Value>(raw).map_err(|e| {
    TimerError::new(
      ErrorCode::InvalidArgument,
      format!("invalid payload JSON: {e}"),
    )
  })
}
