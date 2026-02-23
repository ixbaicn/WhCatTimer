# @whitecat/whcat-timer

面向 Node.js 的高可靠定时调度核心，基于 Rust + napi-rs。

English README: [README.md](./README.md)

## 项目介绍

`@whitecat/whcat-timer` 适用于生产环境下的任务调度场景，重点解决：

- 大量任务下的低开销调度
- 长期运行稳定性
- 进程重启后的任务恢复
- 输入安全校验与可观测性

调度运行时核心由 Rust 实现，Node.js 作为业务集成层。

架构说明见：[ARCHITECTURE.md](./ARCHITECTURE.md)

## 编译方式

### 环境要求

- Node.js（版本范围见 `package.json` engines）
- Rust 工具链
- Windows 平台构建原生模块需要 C++ 工具链（`link.exe`）

### 编译命令

当前平台构建：

```bash
npm run build
```

调试构建：

```bash
npm run build:debug
```

启用 SQLite 持久化构建：

```bash
cargo build --features sqlite
```

### 测试命令

Rust 单元测试：

```bash
cargo test
```

TypeScript 类型检查：

```bash
npx tsc --noEmit
```

JS API 测试：

```bash
npx ava
```

## 接口概览

完整接口文档：

- English API doc：[API_EN.md](./API_EN.md)
- 中文接口文档：[API_ZH.md](./API_ZH.md)

### 核心类

- `new WhCatTimer(config?)`

### 调度创建接口

- `scheduleOnce(runAtMs, payloadJson?, jobId?)`
- `scheduleInterval(everyMs, payloadJson?, jobId?, strictIntervalCycle?, count?)`
- `scheduleCron(cronExpr, tz?, payloadJson?, jobId?, count?)`
- `schedule(input)`（统一调度入口）

### 任务控制接口

- `cancelJob(jobId)`
- `pauseJob(jobId)`
- `resumeJob(jobId)`
- `shutdown()`

### 查询接口

- `listJobs()`
- `getJob(jobId)`
- `getJobSchedule(jobId, count?)`
- `getNextRuns(jobId, count)`

### 追溯接口

- `drainExecutionRecords(limit)` / `queryExecutionRecords(limit)`
- `drainAuditLogs(limit)` / `queryAuditLogs(limit)`

### 恢复接口

- `restore()`

### Cron 工具函数

- `everyMinutes(n)`
- `everyHours(n)`
- `dailyAt(hour, minute)`
- `weeklyOn(dow, hour, minute)`
- `monthlyOn(day, hour, minute)`

## 项目优势

- 高性能：
  - 基于二叉堆的调度结构
  - 每 tick 执行预算限制（防风暴）
  - 执行记录内存上限控制
- 高稳定：
  - worker panic 恢复
  - watchdog 自动重建
  - 时钟跳变检测
- 高准确：
  - 支持一次性、周期、表达式三种调度
  - 周期支持严格周期与自然边界对齐两种语义
- 高可观测：
  - 任务列表/详情快照查询
  - 只读追溯查询（`query*`）
  - 消费式流水查询（`drain*`）
- 可恢复：
  - 支持内存和 SQLite 存储
  - 支持恢复机制

## 最小示例

示例文件：`js/minimal-example.ts`

## 支持项目

如果这个项目对你有帮助，欢迎 Star 收藏仓库。
