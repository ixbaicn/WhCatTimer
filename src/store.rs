use std::collections::HashMap;

use parking_lot::Mutex;

use crate::error::TimerResult;
#[cfg(feature = "sqlite")]
use crate::error::{ErrorCode, TimerError};
use crate::types::{AuditRecord, ExecutionRecord, JobSpec, RuntimeState};

pub trait Store: Send + Sync {
  fn save_job(&self, job: &JobSpec) -> TimerResult<()>;
  fn delete_job(&self, job_id: &str) -> TimerResult<()>;
  fn list_jobs(&self) -> TimerResult<Vec<JobSpec>>;
  fn save_runtime_state(&self, state: &RuntimeState) -> TimerResult<()>;
  fn load_runtime_state(&self) -> TimerResult<Vec<RuntimeState>>;
  fn save_execution_record(&self, record: &ExecutionRecord) -> TimerResult<()>;
  fn load_execution_records(&self, limit: usize) -> TimerResult<Vec<ExecutionRecord>>;
  fn query_execution_records(&self, limit: usize) -> TimerResult<Vec<ExecutionRecord>>;
  fn save_audit_record(&self, record: &AuditRecord) -> TimerResult<()>;
  fn drain_audit_records(&self, limit: usize) -> TimerResult<Vec<AuditRecord>>;
  fn query_audit_records(&self, limit: usize) -> TimerResult<Vec<AuditRecord>>;
}

#[derive(Default)]
pub struct MemoryStore {
  jobs: Mutex<HashMap<String, JobSpec>>,
  states: Mutex<HashMap<String, RuntimeState>>,
  records: Mutex<Vec<ExecutionRecord>>,
  audits: Mutex<Vec<AuditRecord>>,
}

impl MemoryStore {
  pub fn new() -> Self {
    Self::default()
  }
}

impl Store for MemoryStore {
  fn save_job(&self, job: &JobSpec) -> TimerResult<()> {
    self.jobs.lock().insert(job.id.clone(), job.clone());
    Ok(())
  }

  fn delete_job(&self, job_id: &str) -> TimerResult<()> {
    self.jobs.lock().remove(job_id);
    self.states.lock().remove(job_id);
    Ok(())
  }

  fn list_jobs(&self) -> TimerResult<Vec<JobSpec>> {
    Ok(self.jobs.lock().values().cloned().collect())
  }

  fn save_runtime_state(&self, state: &RuntimeState) -> TimerResult<()> {
    self
      .states
      .lock()
      .insert(state.job_id.clone(), state.clone());
    Ok(())
  }

  fn load_runtime_state(&self) -> TimerResult<Vec<RuntimeState>> {
    Ok(self.states.lock().values().cloned().collect())
  }

  fn save_execution_record(&self, record: &ExecutionRecord) -> TimerResult<()> {
    self.records.lock().push(record.clone());
    Ok(())
  }

  fn load_execution_records(&self, limit: usize) -> TimerResult<Vec<ExecutionRecord>> {
    let mut guard = self.records.lock();
    let take = limit.min(guard.len());
    let split_at = guard.len() - take;
    let records = guard.split_off(split_at);
    Ok(records)
  }

  fn query_execution_records(&self, limit: usize) -> TimerResult<Vec<ExecutionRecord>> {
    let guard = self.records.lock();
    let take = limit.min(guard.len());
    let start = guard.len() - take;
    Ok(guard[start..].to_vec())
  }

  fn save_audit_record(&self, record: &AuditRecord) -> TimerResult<()> {
    self.audits.lock().push(record.clone());
    Ok(())
  }

  fn drain_audit_records(&self, limit: usize) -> TimerResult<Vec<AuditRecord>> {
    let mut guard = self.audits.lock();
    let take = limit.min(guard.len());
    let split_at = guard.len() - take;
    let rows = guard.split_off(split_at);
    Ok(rows)
  }

  fn query_audit_records(&self, limit: usize) -> TimerResult<Vec<AuditRecord>> {
    let guard = self.audits.lock();
    let take = limit.min(guard.len());
    let start = guard.len() - take;
    Ok(guard[start..].to_vec())
  }
}

#[cfg(feature = "sqlite")]
pub struct SQLiteStore {
  conn: Mutex<rusqlite::Connection>,
}

#[cfg(feature = "sqlite")]
impl SQLiteStore {
  pub fn new(path: &str, schema_version: u32) -> TimerResult<Self> {
    let conn = rusqlite::Connection::open(path)
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    conn
      .execute_batch(
        "
      CREATE TABLE IF NOT EXISTS metadata (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS jobs (
        id TEXT PRIMARY KEY,
        spec_json TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS runtime_state (
        job_id TEXT PRIMARY KEY,
        state_json TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS execution_records (
        id TEXT PRIMARY KEY,
        job_id TEXT NOT NULL,
        record_json TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS audit_records (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        record_json TEXT NOT NULL
      );
      ",
      )
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;

    conn
      .execute(
        "INSERT OR REPLACE INTO metadata(key, value) VALUES('schema_version', ?1)",
        [schema_version.to_string()],
      )
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;

    Ok(Self {
      conn: Mutex::new(conn),
    })
  }
}

#[cfg(feature = "sqlite")]
impl Store for SQLiteStore {
  fn save_job(&self, job: &JobSpec) -> TimerResult<()> {
    let spec_json = serde_json::to_string(job)
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    self
      .conn
      .lock()
      .execute(
        "INSERT OR REPLACE INTO jobs(id, spec_json) VALUES (?1, ?2)",
        [&job.id, &spec_json],
      )
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    Ok(())
  }

  fn delete_job(&self, job_id: &str) -> TimerResult<()> {
    let conn = self.conn.lock();
    conn
      .execute("DELETE FROM jobs WHERE id = ?1", [job_id])
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    conn
      .execute("DELETE FROM runtime_state WHERE job_id = ?1", [job_id])
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    Ok(())
  }

  fn list_jobs(&self) -> TimerResult<Vec<JobSpec>> {
    let conn = self.conn.lock();
    let mut stmt = conn
      .prepare("SELECT spec_json FROM jobs")
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    let rows = stmt
      .query_map([], |row| {
        let raw: String = row.get(0)?;
        Ok(raw)
      })
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;

    let mut out = Vec::new();
    for row in rows {
      let raw = row.map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      let spec = serde_json::from_str::<JobSpec>(&raw)
        .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      out.push(spec);
    }
    Ok(out)
  }

  fn save_runtime_state(&self, state: &RuntimeState) -> TimerResult<()> {
    let state_json = serde_json::to_string(state)
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    self
      .conn
      .lock()
      .execute(
        "INSERT OR REPLACE INTO runtime_state(job_id, state_json) VALUES (?1, ?2)",
        [&state.job_id, &state_json],
      )
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    Ok(())
  }

  fn load_runtime_state(&self) -> TimerResult<Vec<RuntimeState>> {
    let conn = self.conn.lock();
    let mut stmt = conn
      .prepare("SELECT state_json FROM runtime_state")
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    let rows = stmt
      .query_map([], |row| {
        let raw: String = row.get(0)?;
        Ok(raw)
      })
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;

    let mut out = Vec::new();
    for row in rows {
      let raw = row.map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      let state = serde_json::from_str::<RuntimeState>(&raw)
        .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      out.push(state);
    }
    Ok(out)
  }

  fn save_execution_record(&self, record: &ExecutionRecord) -> TimerResult<()> {
    let raw = serde_json::to_string(record)
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    self
      .conn
      .lock()
      .execute(
        "INSERT OR REPLACE INTO execution_records(id, job_id, record_json) VALUES (?1, ?2, ?3)",
        [&record.id, &record.job_id, &raw],
      )
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    Ok(())
  }

  fn load_execution_records(&self, limit: usize) -> TimerResult<Vec<ExecutionRecord>> {
    let conn = self.conn.lock();
    let mut stmt = conn
      .prepare("SELECT id, record_json FROM execution_records ORDER BY rowid DESC LIMIT ?1")
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    let rows = stmt
      .query_map([limit as i64], |row| {
        let id: String = row.get(0)?;
        let raw: String = row.get(1)?;
        Ok((id, raw))
      })
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;

    let mut out = Vec::new();
    let mut ids = Vec::new();
    for row in rows {
      let (id, raw) = row.map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      let record = serde_json::from_str::<ExecutionRecord>(&raw)
        .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      out.push(record);
      ids.push(id);
    }
    for id in ids {
      conn
        .execute("DELETE FROM execution_records WHERE id = ?1", [id])
        .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    }
    Ok(out)
  }

  fn query_execution_records(&self, limit: usize) -> TimerResult<Vec<ExecutionRecord>> {
    let conn = self.conn.lock();
    let mut stmt = conn
      .prepare("SELECT record_json FROM execution_records ORDER BY rowid DESC LIMIT ?1")
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    let rows = stmt
      .query_map([limit as i64], |row| {
        let raw: String = row.get(0)?;
        Ok(raw)
      })
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    let mut out = Vec::new();
    for row in rows {
      let raw = row.map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      let record = serde_json::from_str::<ExecutionRecord>(&raw)
        .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      out.push(record);
    }
    Ok(out)
  }

  fn save_audit_record(&self, record: &AuditRecord) -> TimerResult<()> {
    let raw = serde_json::to_string(record)
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    self
      .conn
      .lock()
      .execute("INSERT INTO audit_records(record_json) VALUES (?1)", [raw])
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    Ok(())
  }

  fn drain_audit_records(&self, limit: usize) -> TimerResult<Vec<AuditRecord>> {
    let conn = self.conn.lock();
    let mut stmt = conn
      .prepare("SELECT id, record_json FROM audit_records ORDER BY id DESC LIMIT ?1")
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    let rows = stmt
      .query_map([limit as i64], |row| {
        let id: i64 = row.get(0)?;
        let raw: String = row.get(1)?;
        Ok((id, raw))
      })
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    let mut out = Vec::new();
    let mut ids = Vec::new();
    for row in rows {
      let (id, raw) = row.map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      let rec = serde_json::from_str::<AuditRecord>(&raw)
        .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      out.push(rec);
      ids.push(id);
    }
    for id in ids {
      conn
        .execute("DELETE FROM audit_records WHERE id = ?1", [id])
        .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    }
    Ok(out)
  }

  fn query_audit_records(&self, limit: usize) -> TimerResult<Vec<AuditRecord>> {
    let conn = self.conn.lock();
    let mut stmt = conn
      .prepare("SELECT record_json FROM audit_records ORDER BY id DESC LIMIT ?1")
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    let rows = stmt
      .query_map([limit as i64], |row| {
        let raw: String = row.get(0)?;
        Ok(raw)
      })
      .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
    let mut out = Vec::new();
    for row in rows {
      let raw = row.map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      let rec = serde_json::from_str::<AuditRecord>(&raw)
        .map_err(|e| TimerError::new(ErrorCode::StoreError, e.to_string()))?;
      out.push(rec);
    }
    Ok(out)
  }
}
