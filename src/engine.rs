use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::{
  atomic::{AtomicBool, Ordering as AtomicOrdering},
  Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use crate::cron_utils::next_cron_due;
use crate::error::{ErrorCode, TimerError, TimerResult};
use crate::store::Store;
use crate::types::{
  AuditRecord, ExecutionRecord, JobSpec, JobState, JobType, MisfirePolicy, RuntimeState,
  TimerConfig,
};

#[derive(Debug, Clone)]
struct HeapEntry {
  due: DateTime<Utc>,
  sequence: u64,
  job_id: String,
}

impl Eq for HeapEntry {}

impl PartialEq for HeapEntry {
  fn eq(&self, other: &Self) -> bool {
    self.due == other.due && self.sequence == other.sequence && self.job_id == other.job_id
  }
}

impl Ord for HeapEntry {
  fn cmp(&self, other: &Self) -> Ordering {
    other
      .due
      .cmp(&self.due)
      .then_with(|| other.sequence.cmp(&self.sequence))
  }
}

impl PartialOrd for HeapEntry {
  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    Some(self.cmp(other))
  }
}

#[derive(Debug)]
struct EngineState {
  jobs: HashMap<String, JobSpec>,
  runtime_state: HashMap<String, RuntimeState>,
  schedule_heap: BinaryHeap<HeapEntry>,
  execution_records: VecDeque<ExecutionRecord>,
  audit_logs: VecDeque<AuditRecord>,
  sequence: u64,
  boot_mono: Instant,
  boot_wall: DateTime<Utc>,
  last_tick_mono: Instant,
  last_wall: DateTime<Utc>,
}

impl Default for EngineState {
  fn default() -> Self {
    let now = Utc::now();
    let mono = Instant::now();
    Self {
      jobs: HashMap::new(),
      runtime_state: HashMap::new(),
      schedule_heap: BinaryHeap::new(),
      execution_records: VecDeque::new(),
      audit_logs: VecDeque::new(),
      sequence: 0,
      boot_mono: mono,
      boot_wall: now,
      last_tick_mono: mono,
      last_wall: now,
    }
  }
}

pub struct SchedulerEngine {
  config: TimerConfig,
  store: Arc<dyn Store>,
  state: Arc<Mutex<EngineState>>,
  running: Arc<AtomicBool>,
  worker: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug, Clone)]
pub struct JobScheduleSnapshot {
  pub job_type: JobType,
  pub state: JobState,
  pub enabled: bool,
  pub strict_interval_cycle: bool,
  pub max_runs: Option<u32>,
  pub run_count: u32,
  pub last_fired: Option<DateTime<Utc>>,
  pub next_due: Option<DateTime<Utc>>,
  pub next_runs: Vec<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct JobListItemSnapshot {
  pub job_id: String,
  pub job_type: JobType,
  pub state: JobState,
  pub enabled: bool,
  pub strict_interval_cycle: bool,
  pub max_runs: Option<u32>,
  pub run_count: u32,
  pub run_at: Option<DateTime<Utc>>,
  pub every_ms: Option<u64>,
  pub cron_expr: Option<String>,
  pub tz: Option<String>,
  pub last_run: Option<DateTime<Utc>>,
  pub next_run: Option<DateTime<Utc>>,
}

impl SchedulerEngine {
  pub fn new(config: TimerConfig, store: Arc<dyn Store>) -> TimerResult<Self> {
    let engine = Self {
      config,
      store,
      state: Arc::new(Mutex::new(EngineState::default())),
      running: Arc::new(AtomicBool::new(false)),
      worker: Mutex::new(None),
    };
    if engine.config.restore_on_start {
      let _ = engine.restore();
    }
    Ok(engine)
  }

  pub fn start(&self) -> TimerResult<()> {
    if self.running.swap(true, AtomicOrdering::SeqCst) {
      return Ok(());
    }

    let state = Arc::clone(&self.state);
    let store = Arc::clone(&self.store);
    let config = self.config.clone();
    let running = Arc::clone(&self.running);
    let handle = thread::Builder::new()
      .name("whcat-timer-worker".to_string())
      .spawn(move || {
        while running.load(AtomicOrdering::SeqCst) {
          thread::sleep(Duration::from_millis(config.tick_ms));
          let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            process_tick(&state, &store, &config);
            run_watchdog(&state, &store, &config);
          }));
          if result.is_err() {
            rebuild_heap(&state, &store);
            push_audit(&state, &store, "panic_recovered");
          }
        }
      })
      .map_err(|e| TimerError::new(ErrorCode::Internal, e.to_string()))?;
    self.worker.lock().replace(handle);
    Ok(())
  }

  pub fn stop(&self) {
    self.running.store(false, AtomicOrdering::SeqCst);
    if let Some(handle) = self.worker.lock().take() {
      let _ = handle.join();
    }
  }

  pub fn schedule_once(&self, mut job: JobSpec) -> TimerResult<String> {
    if job.schedule.run_at.is_none() {
      return Err(TimerError::new(
        ErrorCode::InvalidArgument,
        "schedule_once requires run_at",
      ));
    }
    job.job_type = JobType::Once;
    self.add_job(job)
  }

  pub fn schedule_interval(&self, mut job: JobSpec) -> TimerResult<String> {
    if job.schedule.every_ms.is_none() {
      return Err(TimerError::new(
        ErrorCode::InvalidArgument,
        "schedule_interval requires every_ms",
      ));
    }
    job.job_type = JobType::Interval;
    self.add_job(job)
  }

  pub fn schedule_cron(&self, mut job: JobSpec) -> TimerResult<String> {
    if job.schedule.cron_expr.is_none() {
      return Err(TimerError::new(
        ErrorCode::InvalidArgument,
        "schedule_cron requires cron_expr",
      ));
    }
    if job.schedule.tz.is_none() {
      job.schedule.tz = Some("UTC".to_string());
    }
    job.job_type = JobType::Cron;
    self.add_job(job)
  }

  fn add_job(&self, job: JobSpec) -> TimerResult<String> {
    validate_job(&job, &self.config)?;
    let next_due = compute_initial_due(&job, Utc::now())?;
    let runtime = RuntimeState::new(job.id.clone(), next_due);

    {
      let mut state = self.state.lock();
      if state.jobs.len() >= self.config.max_jobs {
        return Err(TimerError::new(
          ErrorCode::JobLimitExceeded,
          format!("job count exceeds {}", self.config.max_jobs),
        ));
      }
      state.jobs.insert(job.id.clone(), job.clone());
      state.runtime_state.insert(job.id.clone(), runtime.clone());
      if let Some(due) = next_due {
        push_heap(&mut state, &job.id, due);
      }
    }

    self.store.save_job(&job)?;
    self.store.save_runtime_state(&runtime)?;
    Ok(job.id)
  }

  pub fn cancel_job(&self, job_id: &str) -> TimerResult<bool> {
    let mut state = self.state.lock();
    let exists = state.jobs.remove(job_id).is_some();
    state.runtime_state.remove(job_id);
    drop(state);
    if exists {
      self.store.delete_job(job_id)?;
    }
    Ok(exists)
  }

  pub fn pause_job(&self, job_id: &str) -> TimerResult<()> {
    let mut state = self.state.lock();
    let runtime = state
      .runtime_state
      .get_mut(job_id)
      .ok_or_else(|| TimerError::new(ErrorCode::JobNotFound, format!("job not found: {job_id}")))?;
    runtime.state = JobState::Paused;
    let snapshot = runtime.clone();
    drop(state);
    self.store.save_runtime_state(&snapshot)?;
    Ok(())
  }

  pub fn resume_job(&self, job_id: &str) -> TimerResult<()> {
    let mut state = self.state.lock();
    let job =
      state.jobs.get(job_id).cloned().ok_or_else(|| {
        TimerError::new(ErrorCode::JobNotFound, format!("job not found: {job_id}"))
      })?;
    let runtime = state.runtime_state.get_mut(job_id).ok_or_else(|| {
      TimerError::new(
        ErrorCode::JobNotFound,
        format!("runtime state not found: {job_id}"),
      )
    })?;
    runtime.state = JobState::Scheduled;
    if runtime.next_due.is_none() {
      runtime.next_due = compute_next_due(&job, Utc::now(), runtime.last_fired)?;
    }
    let due = runtime.next_due;
    let snapshot = runtime.clone();
    if let Some(due) = due {
      push_heap(&mut state, job_id, due);
    }
    drop(state);
    self.store.save_runtime_state(&snapshot)?;
    Ok(())
  }

  pub fn get_next_runs(&self, job_id: &str, count: usize) -> TimerResult<Vec<DateTime<Utc>>> {
    let state = self.state.lock();
    let job = state
      .jobs
      .get(job_id)
      .ok_or_else(|| TimerError::new(ErrorCode::JobNotFound, format!("job not found: {job_id}")))?;
    let runtime = state.runtime_state.get(job_id).ok_or_else(|| {
      TimerError::new(
        ErrorCode::JobNotFound,
        format!("runtime not found: {job_id}"),
      )
    })?;
    let mut out = Vec::new();
    let mut pivot = runtime
      .next_due
      .or(runtime.last_fired)
      .unwrap_or_else(Utc::now);
    if let Some(next) = runtime.next_due {
      out.push(next);
      pivot = next;
    }
    while out.len() < count {
      let Some(next) = compute_next_due(job, pivot, Some(pivot))? else {
        break;
      };
      out.push(next);
      pivot = next;
      if job.job_type == JobType::Once {
        break;
      }
    }
    Ok(out)
  }

  pub fn get_job_schedule(&self, job_id: &str, count: usize) -> TimerResult<JobScheduleSnapshot> {
    let state = self.state.lock();
    let job = state
      .jobs
      .get(job_id)
      .ok_or_else(|| TimerError::new(ErrorCode::JobNotFound, format!("job not found: {job_id}")))?;
    let runtime = state.runtime_state.get(job_id).ok_or_else(|| {
      TimerError::new(
        ErrorCode::JobNotFound,
        format!("runtime not found: {job_id}"),
      )
    })?;
    let mut next_runs = Vec::new();
    let mut pivot = runtime
      .next_due
      .or(runtime.last_fired)
      .unwrap_or_else(Utc::now);
    if let Some(next) = runtime.next_due {
      next_runs.push(next);
      pivot = next;
    }
    while next_runs.len() < count {
      let Some(next) = compute_next_due(job, pivot, Some(pivot))? else {
        break;
      };
      next_runs.push(next);
      pivot = next;
      if job.job_type == JobType::Once {
        break;
      }
    }
    Ok(JobScheduleSnapshot {
      job_type: job.job_type.clone(),
      state: runtime.state.clone(),
      enabled: job.enabled,
      strict_interval_cycle: job.policy.strict_interval_cycle,
      max_runs: job.policy.max_runs,
      run_count: runtime.run_count,
      last_fired: runtime.last_fired,
      next_due: runtime.next_due,
      next_runs,
    })
  }

  pub fn get_job(&self, job_id: &str) -> TimerResult<JobListItemSnapshot> {
    let state = self.state.lock();
    let job = state
      .jobs
      .get(job_id)
      .ok_or_else(|| TimerError::new(ErrorCode::JobNotFound, format!("job not found: {job_id}")))?;
    let runtime = state.runtime_state.get(job_id);
    Ok(JobListItemSnapshot {
      job_id: job_id.to_string(),
      job_type: job.job_type.clone(),
      state: runtime
        .map(|r| r.state.clone())
        .unwrap_or(JobState::Scheduled),
      enabled: job.enabled,
      strict_interval_cycle: job.policy.strict_interval_cycle,
      max_runs: job.policy.max_runs,
      run_count: runtime.map(|r| r.run_count).unwrap_or(0),
      run_at: job.schedule.run_at,
      every_ms: job.schedule.every_ms,
      cron_expr: job.schedule.cron_expr.clone(),
      tz: job.schedule.tz.clone(),
      last_run: runtime.and_then(|r| r.last_fired),
      next_run: runtime.and_then(|r| r.next_due),
    })
  }

  pub fn list_jobs(&self) -> Vec<JobListItemSnapshot> {
    let state = self.state.lock();
    let mut out = Vec::with_capacity(state.jobs.len());
    for (job_id, job) in &state.jobs {
      let runtime = state.runtime_state.get(job_id);
      out.push(JobListItemSnapshot {
        job_id: job_id.clone(),
        job_type: job.job_type.clone(),
        state: runtime
          .map(|r| r.state.clone())
          .unwrap_or(JobState::Scheduled),
        enabled: job.enabled,
        strict_interval_cycle: job.policy.strict_interval_cycle,
        max_runs: job.policy.max_runs,
        run_count: runtime.map(|r| r.run_count).unwrap_or(0),
        run_at: job.schedule.run_at,
        every_ms: job.schedule.every_ms,
        cron_expr: job.schedule.cron_expr.clone(),
        tz: job.schedule.tz.clone(),
        last_run: runtime.and_then(|r| r.last_fired),
        next_run: runtime.and_then(|r| r.next_due),
      });
    }
    out.sort_by(|a, b| {
      a.next_run
        .cmp(&b.next_run)
        .then_with(|| a.job_id.cmp(&b.job_id))
    });
    out
  }

  pub fn restore(&self) -> TimerResult<usize> {
    let jobs = self.store.list_jobs()?;
    let runtime = self.store.load_runtime_state()?;
    let runtime_map = runtime
      .into_iter()
      .map(|s| (s.job_id.clone(), s))
      .collect::<HashMap<_, _>>();

    {
      let mut state = self.state.lock();
      state.jobs.clear();
      state.runtime_state.clear();
      state.schedule_heap.clear();

      for job in jobs {
        let mut rt = runtime_map
          .get(&job.id)
          .cloned()
          .unwrap_or_else(|| RuntimeState::new(job.id.clone(), None));

        if rt.next_due.is_none() {
          rt.next_due = compute_initial_due(&job, Utc::now())?;
        }
        if let Some(due) = rt.next_due {
          push_heap(&mut state, &job.id, due);
        }
        state.runtime_state.insert(job.id.clone(), rt);
        state.jobs.insert(job.id.clone(), job);
      }
      append_audit_locked(&mut state, &self.store, "restore_completed");
      Ok(state.jobs.len())
    }
  }

  pub fn drain_execution_records(&self, limit: usize) -> TimerResult<Vec<ExecutionRecord>> {
    let mut state = self.state.lock();
    let mut out = Vec::new();
    let take = limit.min(state.execution_records.len());
    for _ in 0..take {
      if let Some(rec) = state.execution_records.pop_front() {
        out.push(rec);
      }
    }
    if out.is_empty() {
      return self.store.load_execution_records(limit);
    }
    Ok(out)
  }

  pub fn query_execution_records(&self, limit: usize) -> TimerResult<Vec<ExecutionRecord>> {
    self.store.query_execution_records(limit)
  }

  pub fn drain_audit_logs(&self, limit: usize) -> Vec<AuditRecord> {
    self.store.drain_audit_records(limit).unwrap_or_default()
  }

  pub fn query_audit_logs(&self, limit: usize) -> Vec<AuditRecord> {
    self.store.query_audit_records(limit).unwrap_or_default()
  }
}

impl Drop for SchedulerEngine {
  fn drop(&mut self) {
    self.stop();
  }
}

fn push_heap(state: &mut EngineState, job_id: &str, due: DateTime<Utc>) {
  state.sequence += 1;
  state.schedule_heap.push(HeapEntry {
    due,
    sequence: state.sequence,
    job_id: job_id.to_string(),
  });
}

fn push_audit_locked(state: &mut EngineState, event: &str) -> AuditRecord {
  let record = AuditRecord::new(event);
  state.audit_logs.push_back(record.clone());
  if state.audit_logs.len() > 20_000 {
    state.audit_logs.pop_front();
  }
  record
}

fn append_audit_locked(state: &mut EngineState, store: &Arc<dyn Store>, event: &str) {
  let record = push_audit_locked(state, event);
  let _ = store.save_audit_record(&record);
}

fn push_audit(state: &Arc<Mutex<EngineState>>, store: &Arc<dyn Store>, event: &str) {
  let mut guard = state.lock();
  append_audit_locked(&mut guard, store, event);
}

fn rebuild_heap(state: &Arc<Mutex<EngineState>>, store: &Arc<dyn Store>) {
  let mut guard = state.lock();
  guard.schedule_heap.clear();
  let entries = guard
    .runtime_state
    .iter()
    .filter_map(|(job_id, runtime)| runtime.next_due.map(|due| (job_id.clone(), due)))
    .collect::<Vec<_>>();
  for (job_id, due) in entries {
    push_heap(&mut guard, &job_id, due);
  }
  append_audit_locked(&mut guard, store, "heap_rebuilt");
}

fn run_watchdog(state: &Arc<Mutex<EngineState>>, store: &Arc<dyn Store>, config: &TimerConfig) {
  let mut guard = state.lock();
  let elapsed = guard.last_tick_mono.elapsed().as_millis() as u64;
  if elapsed > config.watchdog_timeout_ms {
    guard.schedule_heap.clear();
    let entries = guard
      .runtime_state
      .iter()
      .filter_map(|(job_id, runtime)| runtime.next_due.map(|due| (job_id.clone(), due)))
      .collect::<Vec<_>>();
    for (job_id, due) in entries {
      push_heap(&mut guard, &job_id, due);
    }
    append_audit_locked(&mut guard, store, "watchdog_heap_rebuild");
    guard.last_tick_mono = Instant::now();
  }
}

fn process_tick(state: &Arc<Mutex<EngineState>>, store: &Arc<dyn Store>, config: &TimerConfig) {
  let now = Utc::now();
  let mut guard = state.lock();
  let tick_budget = config.max_global_concurrency.max(1) as usize;

  guard.last_tick_mono = Instant::now();
  let expected_wall =
    guard.boot_wall + chrono::Duration::milliseconds(guard.boot_mono.elapsed().as_millis() as i64);
  let jump = now.signed_duration_since(expected_wall).num_milliseconds();
  if jump.abs() > config.wall_clock_jump_threshold_ms {
    append_audit_locked(&mut guard, store, "wall_clock_jump_detected");
  }
  if now < guard.last_wall {
    append_audit_locked(&mut guard, store, "wall_clock_backward_detected");
  }
  guard.last_wall = now;

  let mut safety = 0usize;
  let mut fired_this_tick = 0usize;
  while let Some(entry) = guard.schedule_heap.peek().cloned() {
    if entry.due > now {
      break;
    }
    if fired_this_tick >= tick_budget {
      append_audit_locked(&mut guard, store, "tick_budget_exhausted");
      break;
    }
    guard.schedule_heap.pop();
    safety += 1;
    if safety > 100_000 {
      guard.schedule_heap.clear();
      append_audit_locked(&mut guard, store, "heap_safety_clear");
      break;
    }
    let Some(job) = guard.jobs.get(&entry.job_id).cloned() else {
      continue;
    };
    let Some(mut runtime) = guard.runtime_state.get(&entry.job_id).cloned() else {
      continue;
    };

    if !job.enabled || runtime.state == JobState::Paused || runtime.state == JobState::Cancelled {
      continue;
    }
    if runtime.next_due != Some(entry.due) {
      continue;
    }

    let mut fire_times = vec![entry.due];
    if job.job_type == JobType::Interval && job.policy.misfire == MisfirePolicy::CatchUp {
      if let Some(every_ms) = job.schedule.every_ms {
        let lag_ms = now
          .signed_duration_since(entry.due)
          .num_milliseconds()
          .max(0) as u64;
        let missed = (lag_ms / every_ms) as u32;
        let catchup = missed.min(job.policy.max_catchup);
        for i in 1..=catchup {
          fire_times.push(entry.due + chrono::Duration::milliseconds((i as u64 * every_ms) as i64));
        }
      }
    }

    if job.policy.misfire == MisfirePolicy::Skip
      && now.signed_duration_since(entry.due).num_milliseconds() > config.tick_ms as i64
    {
      fire_times.clear();
    }

    if let Some(max_runs) = job.policy.max_runs {
      let done = runtime.run_count;
      if done >= max_runs {
        fire_times.clear();
      } else {
        let remaining_runs = (max_runs - done) as usize;
        if fire_times.len() > remaining_runs {
          fire_times.truncate(remaining_runs);
          append_audit_locked(&mut guard, store, "max_runs_clamped");
        }
      }
    }

    let mut deferred_due = None;
    let remaining_budget = tick_budget.saturating_sub(fired_this_tick);
    if fire_times.len() > remaining_budget {
      if remaining_budget == 0 {
        deferred_due = Some(entry.due);
        fire_times.clear();
      } else {
        deferred_due = Some(fire_times[remaining_budget]);
        fire_times.truncate(remaining_budget);
      }
      append_audit_locked(&mut guard, store, "tick_budget_deferred");
    }

    for planned in fire_times {
      runtime.state = JobState::Running;
      runtime.last_fired = Some(planned);
      runtime.run_count = runtime.run_count.saturating_add(1);
      let record = ExecutionRecord::success(&entry.job_id, planned, now);
      guard.execution_records.push_back(record.clone());
      trim_execution_records(&mut guard, config.max_execution_records);
      let _ = store.save_execution_record(&record);
      fired_this_tick += 1;
    }

    let exhausted_runs = job
      .policy
      .max_runs
      .map(|n| runtime.run_count >= n)
      .unwrap_or(false);

    let next = if exhausted_runs {
      None
    } else if let Some(due) = deferred_due {
      Some(due)
    } else {
      compute_next_due(&job, now, runtime.last_fired)
        .ok()
        .flatten()
    };
    runtime.next_due = next;
    runtime.state = if next.is_some() {
      JobState::Scheduled
    } else {
      JobState::Completed
    };

    if exhausted_runs {
      guard.jobs.remove(&entry.job_id);
      guard.runtime_state.remove(&entry.job_id);
      let _ = store.delete_job(&entry.job_id);
      append_audit_locked(&mut guard, store, "max_runs_completed_deleted");
    } else {
      let snapshot = runtime.clone();
      guard.runtime_state.insert(entry.job_id.clone(), runtime);
      let _ = store.save_runtime_state(&snapshot);
      if let Some(next_due) = next {
        push_heap(&mut guard, &entry.job_id, next_due);
      }
    }
  }
}

fn trim_execution_records(state: &mut EngineState, max_len: usize) {
  if max_len == 0 {
    state.execution_records.clear();
    return;
  }
  while state.execution_records.len() > max_len {
    state.execution_records.pop_front();
  }
}

fn validate_job(job: &JobSpec, config: &TimerConfig) -> TimerResult<()> {
  let payload = serde_json::to_vec(&job.payload)
    .map_err(|e| TimerError::new(ErrorCode::InvalidArgument, e.to_string()))?;
  if payload.len() > config.payload_max_bytes {
    return Err(TimerError::new(
      ErrorCode::PayloadTooLarge,
      format!(
        "payload too large: {} > {}",
        payload.len(),
        config.payload_max_bytes
      ),
    ));
  }
  if job.policy.concurrency == 0 || job.policy.concurrency > config.max_global_concurrency {
    return Err(TimerError::new(
      ErrorCode::InvalidArgument,
      "invalid per-job concurrency",
    ));
  }
  Ok(())
}

fn compute_initial_due(job: &JobSpec, now: DateTime<Utc>) -> TimerResult<Option<DateTime<Utc>>> {
  match job.job_type {
    JobType::Once => Ok(job.schedule.run_at),
    JobType::Interval => {
      let every = job.schedule.every_ms.ok_or_else(|| {
        TimerError::new(
          ErrorCode::InvalidArgument,
          "interval schedule missing every_ms",
        )
      })?;
      if job.policy.strict_interval_cycle {
        Ok(Some(now + chrono::Duration::milliseconds(every as i64)))
      } else {
        Ok(Some(next_aligned_due(now, every)?))
      }
    }
    JobType::Cron => {
      let expr = job.schedule.cron_expr.as_deref().ok_or_else(|| {
        TimerError::new(
          ErrorCode::InvalidArgument,
          "cron schedule missing cron_expr",
        )
      })?;
      let tz = job.schedule.tz.as_deref().unwrap_or("UTC");
      let next = next_cron_due(expr, tz, now)?;
      Ok(Some(next))
    }
  }
}

fn compute_next_due(
  job: &JobSpec,
  now: DateTime<Utc>,
  last_fired: Option<DateTime<Utc>>,
) -> TimerResult<Option<DateTime<Utc>>> {
  match job.job_type {
    JobType::Once => Ok(None),
    JobType::Interval => {
      let every = job.schedule.every_ms.ok_or_else(|| {
        TimerError::new(
          ErrorCode::InvalidArgument,
          "interval schedule missing every_ms",
        )
      })?;
      if job.policy.strict_interval_cycle {
        let base = last_fired.unwrap_or(now);
        let mut next = base + chrono::Duration::milliseconds(every as i64);
        while next <= now {
          next += chrono::Duration::milliseconds(every as i64);
        }
        Ok(Some(next))
      } else {
        Ok(Some(next_aligned_due(now, every)?))
      }
    }
    JobType::Cron => {
      let expr = job.schedule.cron_expr.as_deref().ok_or_else(|| {
        TimerError::new(
          ErrorCode::InvalidArgument,
          "cron schedule missing cron_expr",
        )
      })?;
      let tz = job.schedule.tz.as_deref().unwrap_or("UTC");
      let from = last_fired.unwrap_or(now);
      let next = next_cron_due(expr, tz, from)?;
      Ok(Some(next))
    }
  }
}

fn next_aligned_due(now: DateTime<Utc>, every_ms: u64) -> TimerResult<DateTime<Utc>> {
  if every_ms == 0 || every_ms > i64::MAX as u64 {
    return Err(TimerError::new(
      ErrorCode::InvalidArgument,
      "interval every_ms is out of supported range",
    ));
  }
  let interval = every_ms as i64;
  let current = now.timestamp_millis();
  let remainder = current.rem_euclid(interval);
  let delta = if remainder == 0 {
    interval
  } else {
    interval - remainder
  };
  Ok(now + chrono::Duration::milliseconds(delta))
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;

  use chrono::{Duration, Utc};
  use serde_json::json;

  use crate::store::MemoryStore;
  use crate::types::{
    JobPolicy, JobSpec, JobType, MisfirePolicy, RuntimeState, Schedule, StoreKind, TimerConfig,
  };

  use super::{
    compute_next_due, process_tick, push_heap, rebuild_heap, EngineState, SchedulerEngine,
  };

  #[test]
  fn test_schedule_interval_and_next_runs() {
    let config = TimerConfig {
      store_kind: StoreKind::Memory,
      tick_ms: 50,
      ..TimerConfig::default()
    };
    let store = Arc::new(MemoryStore::new());
    let engine = SchedulerEngine::new(config, store).expect("engine");

    let mut job = JobSpec::new(
      JobType::Interval,
      Schedule::interval(1_000),
      json!({"k":"v"}),
    );
    job.id = "test_interval".to_string();
    let id = engine.schedule_interval(job).expect("schedule interval");
    let runs = engine.get_next_runs(&id, 3).expect("get runs");
    assert!(runs.len() >= 2);
    assert!(runs[1] > runs[0]);
  }

  #[test]
  fn test_restore() {
    let config = TimerConfig {
      store_kind: StoreKind::Memory,
      ..TimerConfig::default()
    };
    let store = Arc::new(MemoryStore::new());
    let engine = SchedulerEngine::new(config.clone(), store.clone()).expect("engine");
    let mut once = JobSpec::new(
      JobType::Once,
      Schedule::once(Utc::now() + Duration::seconds(20)),
      json!({"type":"once"}),
    );
    once.id = "restore_once".to_string();
    engine.schedule_once(once).expect("schedule");

    let recovered = SchedulerEngine::new(config, store).expect("recover");
    let count = recovered.restore().expect("restore");
    assert!(count >= 1);
  }

  #[test]
  fn test_long_stability_interval_no_drift() {
    let now = Utc::now() + Duration::days(180);
    let job = JobSpec::new(
      JobType::Interval,
      Schedule::interval(30_000),
      json!({"type":"long-run"}),
    );
    let last_fired = now - Duration::seconds(90);
    let next = compute_next_due(&job, now, Some(last_fired))
      .expect("next due")
      .expect("has next");
    assert_eq!(next, now + Duration::seconds(30));
  }

  #[test]
  fn test_time_jump_forward_skip_misfire() {
    let mut config = TimerConfig::default();
    config.tick_ms = 100;

    let store = Arc::new(MemoryStore::new());
    let state = Arc::new(parking_lot::Mutex::new(EngineState::default()));
    let now = Utc::now();

    let mut job = JobSpec::new(
      JobType::Interval,
      Schedule::interval(1_000),
      json!({"k":"v"}),
    );
    job.id = "misfire_skip".to_string();
    job.policy = JobPolicy {
      misfire: MisfirePolicy::Skip,
      ..JobPolicy::default()
    };

    let mut runtime = RuntimeState::new(job.id.clone(), Some(now - Duration::seconds(5)));
    runtime.next_due = Some(now - Duration::seconds(5));

    {
      let mut guard = state.lock();
      guard.jobs.insert(job.id.clone(), job.clone());
      guard.runtime_state.insert(job.id.clone(), runtime);
      push_heap(&mut guard, &job.id, now - Duration::seconds(5));
    }

    let store_dyn: Arc<dyn crate::store::Store> = store;
    process_tick(&state, &store_dyn, &config);

    let guard = state.lock();
    assert!(guard.execution_records.is_empty());
  }

  #[test]
  fn test_heap_rebuild_self_heal() {
    let state = Arc::new(parking_lot::Mutex::new(EngineState::default()));
    let mut job = JobSpec::new(
      JobType::Interval,
      Schedule::interval(1_000),
      json!({"k":"v"}),
    );
    job.id = "rebuild_job".to_string();

    {
      let mut guard = state.lock();
      guard.jobs.insert(job.id.clone(), job.clone());
      guard.runtime_state.insert(
        job.id.clone(),
        RuntimeState::new(job.id.clone(), Some(Utc::now() + Duration::seconds(10))),
      );
      guard.schedule_heap.clear();
    }

    let store_dyn: Arc<dyn crate::store::Store> = Arc::new(MemoryStore::new());
    rebuild_heap(&state, &store_dyn);
    let guard = state.lock();
    assert!(!guard.schedule_heap.is_empty());
  }

  #[test]
  fn test_max_runs_two_then_delete() {
    let mut config = TimerConfig::default();
    config.tick_ms = 100;
    config.max_global_concurrency = 100;

    let store = Arc::new(MemoryStore::new());
    let state = Arc::new(parking_lot::Mutex::new(EngineState::default()));
    let now = Utc::now();

    let mut job = JobSpec::new(
      JobType::Interval,
      Schedule::interval(1_000),
      json!({"k":"v"}),
    );
    job.id = "max_runs_two".to_string();
    job.policy = JobPolicy {
      misfire: MisfirePolicy::CatchUp,
      max_catchup: 10,
      max_runs: Some(2),
      ..JobPolicy::default()
    };

    let mut runtime = RuntimeState::new(job.id.clone(), Some(now - Duration::seconds(5)));
    runtime.next_due = Some(now - Duration::seconds(5));

    {
      let mut guard = state.lock();
      guard.jobs.insert(job.id.clone(), job.clone());
      guard.runtime_state.insert(job.id.clone(), runtime);
      push_heap(&mut guard, &job.id, now - Duration::seconds(5));
    }

    let store_dyn: Arc<dyn crate::store::Store> = store;
    process_tick(&state, &store_dyn, &config);

    let guard = state.lock();
    assert!(!guard.jobs.contains_key("max_runs_two"));
    assert!(!guard.runtime_state.contains_key("max_runs_two"));
    let fired = guard
      .execution_records
      .iter()
      .filter(|r| r.job_id == "max_runs_two")
      .count();
    assert_eq!(fired, 2);
  }
}
