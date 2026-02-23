# WhCatTimer API Documentation (English)

This document is for third-party developers. It covers all exported APIs, their purpose, required/optional parameters, return values, and practical usage guidance.

## 1. Import

```ts
import {
  WhCatTimer,
  everyMinutes,
  everyHours,
  dailyAt,
  weeklyOn,
  monthlyOn,
} from '@whitecat/whcat-timer'
```

## 2. Core Class: `WhCatTimer`

### 2.1 `new WhCatTimer(config?)`

Creates a scheduler instance and starts the background worker.

#### `config` (optional)

| Field | Type | Required | Description |
|---|---|---|---|
| `tickMs` | `number` | No | Scheduler tick interval in milliseconds |
| `maxJobs` | `number` | No | Maximum number of jobs |
| `payloadMaxBytes` | `number` | No | Max payload size per job |
| `maxGlobalConcurrency` | `number` | No | Per-tick global execution budget |
| `maxExecutionRecords` | `number` | No | In-memory execution record cap |
| `watchdogTimeoutMs` | `number` | No | Watchdog timeout threshold |
| `wallClockJumpThresholdMs` | `number` | No | Wall-clock jump detection threshold |
| `storeKind` | `'memory' \| 'sqlite'` | No | Storage backend |
| `sqlitePath` | `string` | No | SQLite file path when `storeKind='sqlite'` |
| `restoreOnStart` | `boolean` | No | Restore jobs automatically on startup |

---

### 2.2 Scheduling APIs

#### `scheduleOnce(runAtMs, payloadJson?, jobId?)`

Creates a one-time job.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `runAtMs` | `number` | Yes | Absolute execution timestamp in milliseconds |
| `payloadJson` | `string` | No | JSON string payload |
| `jobId` | `string` | No | Custom job id (sanitized for safety) |

Returns: `{ id: string }`

#### `scheduleInterval(everyMs, payloadJson?, jobId?, strictIntervalCycle?, count?)`

Creates an interval job.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `everyMs` | `number` | Yes | Interval in milliseconds |
| `payloadJson` | `string` | No | JSON string payload |
| `jobId` | `string` | No | Custom job id |
| `strictIntervalCycle` | `boolean` | No | `true` = strict interval cycle, `false` = aligned wall-clock boundary |
| `count` | `number` | No | Run count: omitted/0 = unlimited, N = run N times then auto-delete |

Returns: `{ id: string }`

#### `scheduleCron(cronExpr, tz?, payloadJson?, jobId?, count?)`

Creates a cron-based job.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `cronExpr` | `string` | Yes | Cron expression (sanitized + validated) |
| `tz` | `string` | No | Timezone, default `UTC` |
| `payloadJson` | `string` | No | JSON string payload |
| `jobId` | `string` | No | Custom job id |
| `count` | `number` | No | Run count: omitted/0 = unlimited, N = run N times then auto-delete |

Returns: `{ id: string }`

#### `schedule(input)`

Unified scheduling API (recommended for new integrations).

##### `input` fields

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `'once' \| 'interval' \| 'cron'` | Yes | Schedule type |
| `runAtMs` | `number` | Required for `once` | One-time execution timestamp |
| `everyMs` | `number` | Required for `interval` | Interval in milliseconds |
| `cronExpr` | `string` | Required for `cron` | Cron expression |
| `tz` | `string` | No | Timezone (mainly for cron) |
| `payloadJson` | `string` | No | JSON string payload |
| `jobId` | `string` | No | Custom job id |
| `strictIntervalCycle` | `boolean` | No | For interval jobs |
| `count` | `number` | No | Run count policy (same as above) |

Returns: `{ id: string }`

---

### 2.3 Job Control APIs

#### `cancelJob(jobId)`
Cancels a job.

Returns: `boolean` (whether a job existed and was canceled)

#### `pauseJob(jobId)`
Pauses a job.

Returns: `void`

#### `resumeJob(jobId)`
Resumes a paused job.

Returns: `void`

---

### 2.4 Plan/State Query APIs

#### `getNextRuns(jobId, count)`
Returns future execution times as millisecond timestamps.

#### `getJobSchedule(jobId, count?)`
Returns a full scheduling snapshot for one job.

Returned fields:

| Field | Type | Description |
|---|---|---|
| `jobId` | `string` | Job id |
| `jobType` | `string` | `once / interval / cron` |
| `state` | `string` | `scheduled/running/paused/completed/failed/cancelled` |
| `enabled` | `boolean` | Whether enabled |
| `strictIntervalCycle` | `boolean` | Interval cycle mode |
| `maxRuns` | `number \| undefined` | Max run count |
| `runCount` | `number` | Already executed count |
| `lastRunMs` | `number \| undefined` | Last run timestamp |
| `nextRunMs` | `number \| undefined` | Next run timestamp |
| `nextRunsMs` | `number[]` | Future run timestamps |

#### `listJobs()`
Returns all jobs (recommended for frontend refresh loops).

Each item fields:

| Field | Type | Description |
|---|---|---|
| `jobId` | `string` | Job id |
| `jobType` | `string` | Job type |
| `state` | `string` | Current state |
| `enabled` | `boolean` | Enabled flag |
| `strictIntervalCycle` | `boolean` | Interval cycle mode |
| `maxRuns` | `number \| undefined` | Max run count |
| `runCount` | `number` | Already executed count |
| `runAtMs` | `number \| undefined` | Run time for one-time jobs |
| `everyMs` | `number \| undefined` | Interval value |
| `cronExpr` | `string \| undefined` | Cron expression |
| `tz` | `string \| undefined` | Timezone |
| `lastRunMs` | `number \| undefined` | Last run timestamp |
| `nextRunMs` | `number \| undefined` | Next run timestamp |

#### `getJob(jobId)`
Returns one job detail by id (same structure as one `listJobs()` item).

---

### 2.5 Restore and Lifecycle

#### `restore()`
Restores jobs from storage and rebuilds in-memory scheduling state.

Returns: `number` (number of restored jobs)

#### `shutdown()`
Stops the scheduler worker thread.

---

### 2.6 Traceability APIs (Execution/Audit)

#### `drainExecutionRecords(limit)`
Consume-and-remove execution records.

#### `queryExecutionRecords(limit)`
Read-only execution records query (non-destructive).

Execution record fields:

| Field | Type | Description |
|---|---|---|
| `id` | `string` | Record id |
| `jobId` | `string` | Job id |
| `plannedTimeMs` | `number` | Planned execution time |
| `actualTimeMs` | `number` | Actual execution time |
| `status` | `string` | Execution status |
| `error` | `string \| undefined` | Error message |

#### `drainAuditLogs(limit)`
Consume-and-remove audit logs.

#### `queryAuditLogs(limit)`
Read-only audit log query (non-destructive).

Audit log fields:

| Field | Type | Description |
|---|---|---|
| `timestampMs` | `number` | Event time |
| `event` | `string` | Event name |
| `detailJson` | `string` | Event detail as JSON string |

## 3. Top-Level Cron Helpers

### `everyMinutes(n)`
Builds a cron expression for every N minutes.

### `everyHours(n)`
Builds a cron expression for every N hours.

### `dailyAt(hour, minute)`
Builds a daily cron expression at `HH:mm`.

### `weeklyOn(dow, hour, minute)`
Builds a weekly cron expression.

### `monthlyOn(day, hour, minute)`
Builds a monthly cron expression.

## 4. Integration Recommendations

1. Prefer `schedule(input)` for new projects.
2. Use `listJobs()` for frontend list refresh and `getJob(jobId)` for detail views.
3. Use `queryExecutionRecords/queryAuditLogs` for shared observability; reserve `drain*` for single-consumer pipelines.
4. Always call `shutdown()` during service shutdown.
