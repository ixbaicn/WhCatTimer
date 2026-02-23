# WhCatTimer Architecture

## 1. Overview

WhCatTimer is a high-reliability scheduler core for Node.js built with Rust + `napi-rs`.

Core goals:

- High performance for large job volumes (10w+).
- High stability for long-running processes.
- High accuracy for both fixed-time and interval semantics.
- High safety for input validation and anti-storm control.

Public Node APIs are exposed in `src/timer.rs` through `WhCatTimer` and utility functions (`dailyAt`, `everyMinutes`, etc.).

## 2. Module Layout

- `src/lib.rs`
  - Crate entry, exports napi layer.
- `src/timer.rs`
  - N-API adapter layer, parameter validation/sanitization, error conversion.
  - Converts JS calls to internal engine/store operations.
- `src/engine.rs`
  - Core scheduler engine.
  - Owns in-memory runtime state, worker thread, heap scheduling, tick loop, watchdog, and self-healing.
- `src/types.rs`
  - Shared domain model:
    - `JobSpec`, `Schedule`, `JobPolicy`, `RuntimeState`
    - `ExecutionRecord`, `AuditRecord`, `TimerConfig`
- `src/store.rs`
  - Persistence abstraction (`Store` trait) and implementations:
    - `MemoryStore`
    - `SQLiteStore` (feature-gated by `sqlite`)
- `src/cron_utils.rs`
  - Cron parser helpers, normalization and validation, timezone-aware next-run calculation.
- `src/error.rs`
  - Unified typed errors (`TimerError`, `ErrorCode`).

## 3. Runtime Architecture

### 3.1 Components

- API thread (Node -> NAPI call path)
  - Accepts schedule/query/cancel operations.
  - Performs strict argument checks and sanitization.
- Scheduler worker thread (`whcat-timer-worker`)
  - Runs every `tick_ms` (default `200ms`).
  - Scans due jobs from min-heap and executes scheduling transitions.
- Store backend
  - Persists job definitions, runtime states, execution records, and audit records.

### 3.2 In-Memory State (EngineState)

- `jobs: HashMap<String, JobSpec>`
- `runtime_state: HashMap<String, RuntimeState>`
- `schedule_heap: BinaryHeap<HeapEntry>` (priority by due time)
- `execution_records: VecDeque<ExecutionRecord>`
- `audit_logs: VecDeque<AuditRecord>`
- Monotonic/wall-clock checkpoints:
  - `boot_mono`, `boot_wall`, `last_tick_mono`, `last_wall`

### 3.3 Scheduling Flow

1. API creates `JobSpec` with schedule type (`once`/`interval`/`cron`).
2. Engine validates job (`payload`, `concurrency`, limits).
3. Compute initial due time.
4. Save job + runtime state to store.
5. Push due item into heap.
6. Worker tick pops due entries and updates runtime/execution records.
7. Compute next due, requeue if needed.
8. If `max_runs` reached, auto-delete job.

## 4. Scheduling Semantics

WhCatTimer supports 3 schedule types.

### 4.1 `once`

- Single execution at exact `run_at_ms`.
- `compute_next_due` returns `None` after execution.

### 4.2 `interval`

Requires `every_ms`.

Supports two semantics:

- Strict interval cycle (`strict_interval_cycle = true`, default)
  - Uses last fired time + interval.
  - If delayed, catches up to the next valid cycle point without time drift.
- Natural boundary aligned (`strict_interval_cycle = false`)
  - Aligns to wall-clock boundary by interval modulus.
  - Example: 60 minutes aligns to top of hour boundaries.

### 4.3 `cron`

- Uses timezone-aware cron calculation.
- Accepts 5-field or 6-field format.
- Internally normalized to 6 fields with seconds.
- Unsupported syntax (`L/W/#`) is rejected explicitly.

## 5. Misfire and Backpressure

Per-job misfire strategy (`JobPolicy.misfire`):

- `Skip`
  - Missed executions beyond tick tolerance can be dropped.
- `FireOnce`
  - Default; execute once when overdue.
- `CatchUp`
  - Can emit catch-up executions up to `max_catchup`.

Global anti-storm controls:

- Tick budget = `max_global_concurrency` (minimum 1).
- If budget exhausted:
  - emit audit `tick_budget_exhausted`
  - defer remaining due work (`tick_budget_deferred`)
- Heap safety guard (`>100000` loop pops) triggers `heap_safety_clear`.

## 6. Stability and Self-Healing

### 6.1 Panic Recovery

Worker loop wraps tick processing with `catch_unwind`.

On panic:

- Rebuild heap from runtime states.
- Write audit `panic_recovered`.
- Continue running (avoid process crash).

### 6.2 Watchdog

If `last_tick_mono.elapsed()` exceeds `watchdog_timeout_ms`:

- Rebuild heap.
- Audit `watchdog_heap_rebuild`.

### 6.3 Clock Jump Detection

- Compare expected wall time (from monotonic baseline) with actual wall time.
- If jump exceeds `wall_clock_jump_threshold_ms`:
  - Audit `wall_clock_jump_detected`.
- If wall clock moves backward:
  - Audit `wall_clock_backward_detected`.

## 7. Persistence and Recovery

Store trait (`src/store.rs`) covers:

- Jobs and runtime state
- Execution records:
  - `drain` (consume-and-remove)
  - `query` (read-only)
- Audit records:
  - `drain` (consume-and-remove)
  - `query` (read-only)

`restore()` flow:

1. Load persisted jobs.
2. Load persisted runtime states.
3. Rebuild in-memory maps and heap.
4. Backfill missing `next_due`.
5. Emit `restore_completed`.

If `restore_on_start = true`, engine auto-restores during initialization.

## 8. API Layer Design (Node-facing)

Main exported class: `WhCatTimer`.

Key API groups:

- Create/schedule
  - `scheduleOnce`
  - `scheduleInterval`
  - `scheduleCron`
  - `schedule` (unified input)
- Lifecycle controls
  - `cancelJob`
  - `pauseJob`
  - `resumeJob`
  - `shutdown`
- Query and observability
  - `getNextRuns`
  - `getJobSchedule`
  - `listJobs`
  - `getJob`
  - `drainExecutionRecords`
  - `queryExecutionRecords`
  - `drainAuditLogs`
  - `queryAuditLogs`

Design principle:

- All user-visible errors are returned as JS errors.
- No panic is intentionally propagated to crash Node process.

## 9. Input Validation and Safety

Current safeguards include:

- `count` must be non-negative integer (`0` means unlimited).
- `job_id` sanitization:
  - Allowed: `[A-Za-z0-9_:\\-.]`
  - Empty-after-filter and oversize rejected.
- `cron_expr` sanitization:
  - Keeps digits and cron operators (`* / , -`) + spaces.
- timezone sanitization:
  - Restricted char set and max length checks.
- Payload size cap (`payload_max_bytes`).
- Max job cap (`max_jobs`).

## 10. Performance Characteristics

Primary complexity:

- Schedule insert: `O(log n)` via heap push.
- Due extraction: `O(log n)` per fired/deferred entry.
- Job lookup/update: expected `O(1)` via hash maps.

Memory controls:

- `max_jobs` upper bound.
- In-memory execution record queue trimmed by `max_execution_records`.
- Audit queue capped (currently 20,000 in-memory) and persisted in store.

## 11. Extensibility

Recommended extension points:

- Add new `Store` backend (e.g., Redis/PostgreSQL) by implementing `Store` trait.
- Add executor callback bridge (currently records execution metadata only).
- Add job partitioning/sharding for multi-thread scale-out.
- Add richer audit `detail` fields for full traceability.

## 12. Known Boundaries

- Core scheduler currently runs in a single worker thread.
- Job execution model focuses on scheduling/accounting; business callback orchestration should be integrated at the Node layer or future executor layer.
- SQLite backend depends on `sqlite` feature enablement at build time.

