# WhCatTimer

[![CI](https://github.com/ixbaicn/WhCatTimer/actions/workflows/CI.yml/badge.svg?branch=main)](https://github.com/ixbaicn/WhCatTimer/actions/workflows/CI.yml)

High-reliability timer/scheduler core for Node.js, powered by Rust + napi-rs.

中文版: [README.zh-CN.md](./README.zh-CN.md)

## Project Introduction

`WhCatTimer` is designed for production-grade scheduling scenarios where you need:

- low-overhead scheduling for large job sets
- stable long-running behavior
- recoverable jobs after restart
- strong input validation and operational visibility

The scheduler runtime is implemented in Rust. Node.js is used as the integration surface.

Architecture details: [ARCHITECTURE.md](./ARCHITECTURE.md)

## Build

### Prerequisites

- Node.js (same version range as `package.json` engines)
- Rust toolchain
- Windows users: C++ toolchain (`link.exe`) if building native binaries

### Build Commands

Build for current platform:

```bash
npm run build
```

Debug build:

```bash
npm run build:debug
```

Build with SQLite persistence:

```bash
cargo build --features sqlite
```

### Test Commands

Rust tests:

```bash
cargo test
```

Type check:

```bash
npx tsc --noEmit
```

JS API tests:

```bash
npx ava
```

## API Overview

Complete API reference:

- English API doc: [API_EN.md](./API_EN.md)
- 中文接口文档: [API_ZH.md](./API_ZH.md)

### Core Class

- `new WhCatTimer(config?)`

### Scheduling APIs

- `scheduleOnce(runAtMs, payloadJson?, jobId?)`
- `scheduleInterval(everyMs, payloadJson?, jobId?, strictIntervalCycle?, count?)`
- `scheduleCron(cronExpr, tz?, payloadJson?, jobId?, count?)`
- `schedule(input)` (unified scheduling entry)

### Job Control APIs

- `cancelJob(jobId)`
- `pauseJob(jobId)`
- `resumeJob(jobId)`
- `shutdown()`

### Query APIs

- `listJobs()`
- `getJob(jobId)`
- `getJobSchedule(jobId, count?)`
- `getNextRuns(jobId, count)`

### Traceability APIs

- `drainExecutionRecords(limit)` / `queryExecutionRecords(limit)`
- `drainAuditLogs(limit)` / `queryAuditLogs(limit)`

### Restore API

- `restore()`

### Cron Helper Functions

- `everyMinutes(n)`
- `everyHours(n)`
- `dailyAt(hour, minute)`
- `weeklyOn(dow, hour, minute)`
- `monthlyOn(day, hour, minute)`

## Project Advantages

- High performance:
  - binary-heap based scheduler
  - bounded tick execution budget
  - in-memory execution record cap
- High stability:
  - worker panic recovery
  - watchdog heap rebuild
  - clock-jump detection
- High accuracy:
  - one-time, interval, and cron semantics
  - strict interval cycle vs aligned wall-clock cycle
- High observability:
  - list/get job snapshots
  - readonly trace queries (`query*`)
  - consumable pipelines (`drain*`)
- Persistence and recovery:
  - memory or SQLite store
  - restore support on startup/runtime

## Minimal Example

See `js/minimal-example.ts`.

## Support This Project

If this project helps your team, please star the repository.
