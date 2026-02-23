use std::str::FromStr;
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::cron_utils;
use crate::engine::SchedulerEngine;
use crate::error::{ErrorCode, TimerError};
use crate::store::{MemoryStore, Store};
use crate::types::{parse_payload, JobSpec, JobState, JobType, Schedule, StoreKind, TimerConfig};

#[napi(object)]
pub struct TimerConfigInput {
  pub tick_ms: Option<u32>,
  pub max_jobs: Option<u32>,
  pub payload_max_bytes: Option<u32>,
  pub max_global_concurrency: Option<u32>,
  pub max_execution_records: Option<u32>,
  pub watchdog_timeout_ms: Option<u32>,
  pub wall_clock_jump_threshold_ms: Option<i32>,
  pub store_kind: Option<String>,
  pub sqlite_path: Option<String>,
  pub restore_on_start: Option<bool>,
}

#[napi(object)]
pub struct ScheduleResult {
  pub id: String,
}

#[napi(object)]
pub struct NapiExecutionRecord {
  pub id: String,
  pub job_id: String,
  pub planned_time_ms: i64,
  pub actual_time_ms: i64,
  pub status: String,
  pub error: Option<String>,
}

#[napi(object)]
pub struct NapiAuditRecord {
  pub timestamp_ms: i64,
  pub event: String,
  pub detail_json: String,
}

#[napi(object)]
pub struct NapiJobSchedule {
  pub job_id: String,
  pub job_type: String,
  pub state: String,
  pub enabled: bool,
  pub strict_interval_cycle: bool,
  pub max_runs: Option<u32>,
  pub run_count: u32,
  pub last_run_ms: Option<i64>,
  pub next_run_ms: Option<i64>,
  pub next_runs_ms: Vec<i64>,
}

#[napi(object)]
pub struct NapiJobListItem {
  pub job_id: String,
  pub job_type: String,
  pub state: String,
  pub enabled: bool,
  pub strict_interval_cycle: bool,
  pub max_runs: Option<u32>,
  pub run_count: u32,
  pub run_at_ms: Option<i64>,
  pub every_ms: Option<u32>,
  pub cron_expr: Option<String>,
  pub tz: Option<String>,
  pub last_run_ms: Option<i64>,
  pub next_run_ms: Option<i64>,
}

#[napi(object)]
pub struct NapiScheduleInput {
  pub r#type: String,
  pub run_at_ms: Option<i64>,
  pub every_ms: Option<u32>,
  pub cron_expr: Option<String>,
  pub tz: Option<String>,
  pub payload_json: Option<String>,
  pub job_id: Option<String>,
  pub strict_interval_cycle: Option<bool>,
  pub count: Option<i64>,
}

#[napi]
pub struct WhCatTimer {
  engine: SchedulerEngine,
}

fn to_js_error(err: TimerError) -> napi::Error {
  napi::Error::new(
    Status::GenericFailure,
    format!("{}: {}", err.code.as_str(), err.message),
  )
}

fn build_store(cfg: &TimerConfig) -> Result<Arc<dyn Store>> {
  match cfg.store_kind {
    StoreKind::Memory => Ok(Arc::new(MemoryStore::new())),
    StoreKind::Sqlite => {
      #[cfg(feature = "sqlite")]
      {
        let path = cfg.sqlite_path.as_deref().ok_or_else(|| {
          to_js_error(TimerError::new(
            ErrorCode::InvalidArgument,
            "sqlite_path is required when store_kind=sqlite",
          ))
        })?;
        let store =
          crate::store::SQLiteStore::new(path, cfg.schema_version).map_err(to_js_error)?;
        return Ok(Arc::new(store));
      }
      #[cfg(not(feature = "sqlite"))]
      {
        Err(to_js_error(TimerError::new(
          ErrorCode::InvalidArgument,
          "sqlite feature is not enabled in this build",
        )))
      }
    }
  }
}

fn build_config(input: Option<TimerConfigInput>) -> Result<TimerConfig> {
  let mut cfg = TimerConfig::default();
  if let Some(input) = input {
    if let Some(v) = input.tick_ms {
      cfg.tick_ms = v as u64;
    }
    if let Some(v) = input.max_jobs {
      cfg.max_jobs = v as usize;
    }
    if let Some(v) = input.payload_max_bytes {
      cfg.payload_max_bytes = v as usize;
    }
    if let Some(v) = input.max_global_concurrency {
      cfg.max_global_concurrency = v;
    }
    if let Some(v) = input.max_execution_records {
      cfg.max_execution_records = v as usize;
    }
    if let Some(v) = input.watchdog_timeout_ms {
      cfg.watchdog_timeout_ms = v as u64;
    }
    if let Some(v) = input.wall_clock_jump_threshold_ms {
      cfg.wall_clock_jump_threshold_ms = v as i64;
    }
    if let Some(v) = input.store_kind {
      cfg.store_kind = StoreKind::from_str(&v).map_err(to_js_error)?;
    }
    cfg.sqlite_path = input.sqlite_path;
    if let Some(v) = input.restore_on_start {
      cfg.restore_on_start = v;
    }
  }
  Ok(cfg)
}

fn must_dt(ms: i64, label: &str) -> Result<chrono::DateTime<Utc>> {
  Utc.timestamp_millis_opt(ms).single().ok_or_else(|| {
    to_js_error(TimerError::new(
      ErrorCode::InvalidArgument,
      format!("invalid timestamp for {label}: {ms}"),
    ))
  })
}

fn parse_run_count(raw: Option<i64>) -> Result<Option<u32>> {
  let Some(value) = raw else {
    return Ok(None);
  };
  if value < 0 {
    return Err(to_js_error(TimerError::new(
      ErrorCode::InvalidArgument,
      "count must be a non-negative integer",
    )));
  }
  if value == 0 {
    return Ok(None);
  }
  if value > u32::MAX as i64 {
    return Err(to_js_error(TimerError::new(
      ErrorCode::InvalidArgument,
      "count is too large",
    )));
  }
  Ok(Some(value as u32))
}

fn sanitize_job_id(input: Option<String>) -> Result<Option<String>> {
  let Some(raw) = input else {
    return Ok(None);
  };
  let cleaned = raw
    .chars()
    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
    .collect::<String>();
  let trimmed = cleaned.trim().to_string();
  if trimmed.is_empty() {
    return Err(to_js_error(TimerError::new(
      ErrorCode::InvalidArgument,
      "job_id is empty after filtering unsafe characters",
    )));
  }
  if trimmed.len() > 128 {
    return Err(to_js_error(TimerError::new(
      ErrorCode::InvalidArgument,
      "job_id length must be <= 128 after filtering",
    )));
  }
  Ok(Some(trimmed))
}

fn sanitize_cron_expr(expr: String) -> Result<String> {
  let cleaned = expr
    .chars()
    .filter_map(|c| {
      if c.is_ascii_digit() || matches!(c, '*' | '/' | ',' | '-') {
        Some(c)
      } else if c.is_ascii_whitespace() {
        Some(' ')
      } else {
        None
      }
    })
    .collect::<String>();
  let normalized = cleaned
    .split_whitespace()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
  if normalized.is_empty() {
    return Err(to_js_error(TimerError::new(
      ErrorCode::InvalidArgument,
      "cron_expr is empty after filtering unsafe characters",
    )));
  }
  Ok(normalized)
}

fn sanitize_timezone(tz: Option<String>) -> Result<Option<String>> {
  let Some(raw) = tz else {
    return Ok(None);
  };
  let cleaned = raw
    .chars()
    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '+' | '-' | '.'))
    .collect::<String>();
  let trimmed = cleaned.trim().to_string();
  if trimmed.is_empty() {
    return Err(to_js_error(TimerError::new(
      ErrorCode::InvalidArgument,
      "tz is empty after filtering unsafe characters",
    )));
  }
  if trimmed.len() > 128 {
    return Err(to_js_error(TimerError::new(
      ErrorCode::InvalidArgument,
      "tz length must be <= 128 after filtering",
    )));
  }
  Ok(Some(trimmed))
}

fn job_type_label(job_type: &JobType) -> String {
  match job_type {
    JobType::Once => "once".to_string(),
    JobType::Interval => "interval".to_string(),
    JobType::Cron => "cron".to_string(),
  }
}

fn job_state_label(state: &JobState) -> String {
  match state {
    JobState::Scheduled => "scheduled".to_string(),
    JobState::Running => "running".to_string(),
    JobState::Paused => "paused".to_string(),
    JobState::Completed => "completed".to_string(),
    JobState::Failed => "failed".to_string(),
    JobState::Cancelled => "cancelled".to_string(),
  }
}

fn to_napi_job_item(v: crate::engine::JobListItemSnapshot) -> NapiJobListItem {
  NapiJobListItem {
    job_id: v.job_id,
    job_type: job_type_label(&v.job_type),
    state: job_state_label(&v.state),
    enabled: v.enabled,
    strict_interval_cycle: v.strict_interval_cycle,
    max_runs: v.max_runs,
    run_count: v.run_count,
    run_at_ms: v.run_at.map(|t| t.timestamp_millis()),
    every_ms: v.every_ms.and_then(|n| u32::try_from(n).ok()),
    cron_expr: v.cron_expr,
    tz: v.tz,
    last_run_ms: v.last_run.map(|t| t.timestamp_millis()),
    next_run_ms: v.next_run.map(|t| t.timestamp_millis()),
  }
}

#[napi]
impl WhCatTimer {
  #[napi(constructor)]
  pub fn new(config: Option<TimerConfigInput>) -> Result<Self> {
    let cfg = build_config(config)?;
    let store = build_store(&cfg)?;
    let engine = SchedulerEngine::new(cfg, store).map_err(to_js_error)?;
    engine.start().map_err(to_js_error)?;
    Ok(Self { engine })
  }

  #[napi(js_name = "scheduleOnce")]
  pub fn schedule_once(
    &self,
    run_at_ms: i64,
    payload_json: Option<String>,
    job_id: Option<String>,
  ) -> Result<ScheduleResult> {
    let run_at = must_dt(run_at_ms, "run_at_ms")?;
    let payload = parse_payload(payload_json.as_deref()).map_err(to_js_error)?;
    let mut job = JobSpec::new(JobType::Once, Schedule::once(run_at), payload);
    if let Some(id) = sanitize_job_id(job_id)? {
      job.id = id;
    }
    let id = self.engine.schedule_once(job).map_err(to_js_error)?;
    Ok(ScheduleResult { id })
  }

  #[napi(js_name = "scheduleInterval")]
  pub fn schedule_interval(
    &self,
    every_ms: u32,
    payload_json: Option<String>,
    job_id: Option<String>,
    strict_interval_cycle: Option<bool>,
    count: Option<i64>,
  ) -> Result<ScheduleResult> {
    let payload = parse_payload(payload_json.as_deref()).map_err(to_js_error)?;
    let mut job = JobSpec::new(
      JobType::Interval,
      Schedule::interval(every_ms as u64),
      payload,
    );
    job.policy.strict_interval_cycle = strict_interval_cycle.unwrap_or(true);
    job.policy.max_runs = parse_run_count(count)?;
    if let Some(id) = sanitize_job_id(job_id)? {
      job.id = id;
    }
    let id = self.engine.schedule_interval(job).map_err(to_js_error)?;
    Ok(ScheduleResult { id })
  }

  #[napi(js_name = "scheduleCron")]
  pub fn schedule_cron(
    &self,
    cron_expr: String,
    tz: Option<String>,
    payload_json: Option<String>,
    job_id: Option<String>,
    count: Option<i64>,
  ) -> Result<ScheduleResult> {
    let cron_expr = sanitize_cron_expr(cron_expr)?;
    let timezone = sanitize_timezone(tz)?.unwrap_or_else(|| "UTC".to_string());
    let _ = cron_utils::normalize_and_validate_cron(&cron_expr).map_err(to_js_error)?;
    let payload = parse_payload(payload_json.as_deref()).map_err(to_js_error)?;
    let mut job = JobSpec::new(JobType::Cron, Schedule::cron(cron_expr, timezone), payload);
    job.policy.max_runs = parse_run_count(count)?;
    if let Some(id) = sanitize_job_id(job_id)? {
      job.id = id;
    }
    let id = self.engine.schedule_cron(job).map_err(to_js_error)?;
    Ok(ScheduleResult { id })
  }

  #[napi]
  pub fn schedule(&self, input: NapiScheduleInput) -> Result<ScheduleResult> {
    match input.r#type.trim().to_ascii_lowercase().as_str() {
      "once" => self.schedule_once(
        input
          .run_at_ms
          .ok_or_else(|| to_js_error(TimerError::new(ErrorCode::InvalidArgument, "run_at_ms is required for type=once")))?,
        input.payload_json,
        input.job_id,
      ),
      "interval" => self.schedule_interval(
        input
          .every_ms
          .ok_or_else(|| to_js_error(TimerError::new(ErrorCode::InvalidArgument, "every_ms is required for type=interval")))?,
        input.payload_json,
        input.job_id,
        input.strict_interval_cycle,
        input.count,
      ),
      "cron" => self.schedule_cron(
        input
          .cron_expr
          .ok_or_else(|| to_js_error(TimerError::new(ErrorCode::InvalidArgument, "cron_expr is required for type=cron")))?,
        input.tz,
        input.payload_json,
        input.job_id,
        input.count,
      ),
      _ => Err(to_js_error(TimerError::new(
        ErrorCode::InvalidArgument,
        "type must be one of once|interval|cron",
      ))),
    }
  }

  #[napi(js_name = "cancelJob")]
  pub fn cancel_job(&self, job_id: String) -> Result<bool> {
    self.engine.cancel_job(&job_id).map_err(to_js_error)
  }

  #[napi(js_name = "pauseJob")]
  pub fn pause_job(&self, job_id: String) -> Result<()> {
    self.engine.pause_job(&job_id).map_err(to_js_error)
  }

  #[napi(js_name = "resumeJob")]
  pub fn resume_job(&self, job_id: String) -> Result<()> {
    self.engine.resume_job(&job_id).map_err(to_js_error)
  }

  #[napi(js_name = "getNextRuns")]
  pub fn get_next_runs(&self, job_id: String, count: u32) -> Result<Vec<i64>> {
    let rows = self
      .engine
      .get_next_runs(&job_id, count as usize)
      .map_err(to_js_error)?;
    Ok(rows.into_iter().map(|v| v.timestamp_millis()).collect())
  }

  #[napi(js_name = "getJobSchedule")]
  pub fn get_job_schedule(&self, job_id: String, count: Option<u32>) -> Result<NapiJobSchedule> {
    let snap = self
      .engine
      .get_job_schedule(&job_id, count.unwrap_or(5) as usize)
      .map_err(to_js_error)?;
    Ok(NapiJobSchedule {
      job_id,
      job_type: job_type_label(&snap.job_type),
      state: job_state_label(&snap.state),
      enabled: snap.enabled,
      strict_interval_cycle: snap.strict_interval_cycle,
      max_runs: snap.max_runs,
      run_count: snap.run_count,
      last_run_ms: snap.last_fired.map(|t| t.timestamp_millis()),
      next_run_ms: snap.next_due.map(|t| t.timestamp_millis()),
      next_runs_ms: snap
        .next_runs
        .into_iter()
        .map(|v| v.timestamp_millis())
        .collect(),
    })
  }

  #[napi(js_name = "listJobs")]
  pub fn list_jobs(&self) -> Result<Vec<NapiJobListItem>> {
    Ok(self.engine.list_jobs().into_iter().map(to_napi_job_item).collect())
  }

  #[napi(js_name = "getJob")]
  pub fn get_job(&self, job_id: String) -> Result<NapiJobListItem> {
    self
      .engine
      .get_job(&job_id)
      .map(to_napi_job_item)
      .map_err(to_js_error)
  }

  #[napi]
  pub fn restore(&self) -> Result<u32> {
    self.engine.restore().map(|n| n as u32).map_err(to_js_error)
  }

  #[napi(js_name = "drainExecutionRecords")]
  pub fn drain_execution_records(&self, limit: u32) -> Result<Vec<NapiExecutionRecord>> {
    let rows = self
      .engine
      .drain_execution_records(limit as usize)
      .map_err(to_js_error)?;
    Ok(
      rows
        .into_iter()
        .map(|r| NapiExecutionRecord {
          id: r.id,
          job_id: r.job_id,
          planned_time_ms: r.planned_time.timestamp_millis(),
          actual_time_ms: r.actual_time.timestamp_millis(),
          status: format!("{:?}", r.status).to_lowercase(),
          error: r.error,
        })
        .collect(),
    )
  }

  #[napi(js_name = "queryExecutionRecords")]
  pub fn query_execution_records(&self, limit: u32) -> Result<Vec<NapiExecutionRecord>> {
    let rows = self
      .engine
      .query_execution_records(limit as usize)
      .map_err(to_js_error)?;
    Ok(
      rows
        .into_iter()
        .map(|r| NapiExecutionRecord {
          id: r.id,
          job_id: r.job_id,
          planned_time_ms: r.planned_time.timestamp_millis(),
          actual_time_ms: r.actual_time.timestamp_millis(),
          status: format!("{:?}", r.status).to_lowercase(),
          error: r.error,
        })
        .collect(),
    )
  }

  #[napi(js_name = "drainAuditLogs")]
  pub fn drain_audit_logs(&self, limit: u32) -> Result<Vec<NapiAuditRecord>> {
    Ok(
      self
        .engine
        .drain_audit_logs(limit as usize)
        .into_iter()
        .map(|r| NapiAuditRecord {
          timestamp_ms: r.timestamp.timestamp_millis(),
          event: r.event,
          detail_json: serde_json::to_string(&r.detail).unwrap_or_else(|_| "{}".to_string()),
        })
        .collect(),
    )
  }

  #[napi(js_name = "queryAuditLogs")]
  pub fn query_audit_logs(&self, limit: u32) -> Result<Vec<NapiAuditRecord>> {
    Ok(
      self
        .engine
        .query_audit_logs(limit as usize)
        .into_iter()
        .map(|r| NapiAuditRecord {
          timestamp_ms: r.timestamp.timestamp_millis(),
          event: r.event,
          detail_json: serde_json::to_string(&r.detail).unwrap_or_else(|_| "{}".to_string()),
        })
        .collect(),
    )
  }

  #[napi]
  pub fn shutdown(&self) {
    self.engine.stop();
  }
}

#[napi]
pub fn every_minutes(n: u32) -> Result<String> {
  cron_utils::every_minutes(n).map_err(to_js_error)
}

#[napi]
pub fn every_hours(n: u32) -> Result<String> {
  cron_utils::every_hours(n).map_err(to_js_error)
}

#[napi]
pub fn daily_at(hour: u32, minute: u32) -> Result<String> {
  cron_utils::daily_at(hour, minute).map_err(to_js_error)
}

#[napi]
pub fn weekly_on(dow: u32, hour: u32, minute: u32) -> Result<String> {
  cron_utils::weekly_on(dow, hour, minute).map_err(to_js_error)
}

#[napi]
pub fn monthly_on(day: u32, hour: u32, minute: u32) -> Result<String> {
  cron_utils::monthly_on(day, hour, minute).map_err(to_js_error)
}
