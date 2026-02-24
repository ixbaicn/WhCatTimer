# WhCatTimer API 文档（中文）

面向第三方开发者的完整接口说明。本文档覆盖当前模块对外提供的所有 API，包括用途、参数（必填/选填）、返回值和示例。

## 1. 模块导入

```ts
import {
  WhCatTimer,
  everyMinutes,
  everyHours,
  dailyAt,
  weeklyOn,
  monthlyOn,
} from 'WhCatTimer'
```

## 2. 核心类：`WhCatTimer`

### 2.1 `new WhCatTimer(config?)`

创建调度器实例并启动后台调度线程。

#### 参数：`config`（选填）

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `tickMs` | `number` | 否 | 调度轮询周期（毫秒） |
| `maxJobs` | `number` | 否 | 最大任务数限制 |
| `payloadMaxBytes` | `number` | 否 | 单任务 payload 最大字节数 |
| `maxGlobalConcurrency` | `number` | 否 | 每个 tick 的全局执行预算（防风暴） |
| `maxExecutionRecords` | `number` | 否 | 内存执行记录上限 |
| `watchdogTimeoutMs` | `number` | 否 | watchdog 判定超时阈值 |
| `wallClockJumpThresholdMs` | `number` | 否 | 系统时钟跳变检测阈值 |
| `storeKind` | `'memory' \| 'sqlite'` | 否 | 存储类型 |
| `sqlitePath` | `string` | 否 | `storeKind='sqlite'` 时 SQLite 文件路径 |
| `restoreOnStart` | `boolean` | 否 | 构造时是否自动恢复任务 |

---

### 2.2 调度创建接口

#### `scheduleOnce(runAtMs, payloadJson?, jobId?)`

创建一次性任务（到点执行一次）。

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `runAtMs` | `number` | 是 | 绝对执行时间戳（毫秒） |
| `payloadJson` | `string` | 否 | JSON 字符串 payload |
| `jobId` | `string` | 否 | 自定义任务 ID（会做安全字符过滤） |

返回：`{ id: string }`

#### `scheduleInterval(everyMs, payloadJson?, jobId?, strictIntervalCycle?, count?)`

创建周期任务。

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `everyMs` | `number` | 是 | 周期间隔（毫秒） |
| `payloadJson` | `string` | 否 | JSON 字符串 payload |
| `jobId` | `string` | 否 | 自定义任务 ID |
| `strictIntervalCycle` | `boolean` | 否 | `true`=严格周期，`false`=自然边界对齐 |
| `count` | `number` | 否 | 执行次数：不传/0=无限次，N=执行 N 次后自动销毁 |

返回：`{ id: string }`

#### `scheduleCron(cronExpr, tz?, payloadJson?, jobId?, count?)`

创建 Cron 表达式任务。

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `cronExpr` | `string` | 是 | Cron 表达式（会先做过滤和校验） |
| `tz` | `string` | 否 | 时区（默认 `UTC`） |
| `payloadJson` | `string` | 否 | JSON 字符串 payload |
| `jobId` | `string` | 否 | 自定义任务 ID |
| `count` | `number` | 否 | 执行次数：不传/0=无限次，N=执行 N 次后自动销毁 |

返回：`{ id: string }`

#### `schedule(input)`

统一调度入口（推荐给新接入）。

##### `input` 字段

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `type` | `'once' \| 'interval' \| 'cron'` | 是 | 调度类型 |
| `runAtMs` | `number` | `once` 必填 | 一次性任务执行时间 |
| `everyMs` | `number` | `interval` 必填 | 周期间隔 |
| `cronExpr` | `string` | `cron` 必填 | Cron 表达式 |
| `tz` | `string` | 否 | 时区，`cron` 常用 |
| `payloadJson` | `string` | 否 | JSON 字符串 payload |
| `jobId` | `string` | 否 | 自定义任务 ID |
| `strictIntervalCycle` | `boolean` | 否 | 仅 `interval` 使用 |
| `count` | `number` | 否 | 执行次数规则同上 |

返回：`{ id: string }`

---

### 2.3 任务控制接口

#### `cancelJob(jobId)`
取消任务。

返回：`boolean`（是否找到并取消）

#### `pauseJob(jobId)`
暂停任务。

返回：`void`

#### `resumeJob(jobId)`
恢复任务。

返回：`void`

---

### 2.4 计划与状态查询接口

#### `getNextRuns(jobId, count)`
获取未来触发时间列表（毫秒时间戳数组）。

#### `getJobSchedule(jobId, count?)`
获取单任务完整调度快照。

返回字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `jobId` | `string` | 任务 ID |
| `jobType` | `string` | `once / interval / cron` |
| `state` | `string` | `scheduled/running/paused/completed/failed/cancelled` |
| `enabled` | `boolean` | 是否启用 |
| `strictIntervalCycle` | `boolean` | interval 周期语义 |
| `maxRuns` | `number \| undefined` | 最大执行次数 |
| `runCount` | `number` | 已执行次数 |
| `lastRunMs` | `number \| undefined` | 上次执行时间 |
| `nextRunMs` | `number \| undefined` | 下次执行时间 |
| `nextRunsMs` | `number[]` | 后续执行时间列表 |

#### `listJobs()`
获取全部任务列表（适合前端列表刷新）。

每项返回字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `jobId` | `string` | 任务 ID |
| `jobType` | `string` | 任务类型 |
| `state` | `string` | 当前状态 |
| `enabled` | `boolean` | 是否启用 |
| `strictIntervalCycle` | `boolean` | interval 周期语义 |
| `maxRuns` | `number \| undefined` | 最大执行次数 |
| `runCount` | `number` | 已执行次数 |
| `runAtMs` | `number \| undefined` | once 触发时间 |
| `everyMs` | `number \| undefined` | interval 周期 |
| `cronExpr` | `string \| undefined` | cron 表达式 |
| `tz` | `string \| undefined` | 时区 |
| `lastRunMs` | `number \| undefined` | 上次执行时间 |
| `nextRunMs` | `number \| undefined` | 下次执行时间 |

#### `getJob(jobId)`
按 `jobId` 获取单任务详情（字段同 `listJobs` 单项）。

---

### 2.5 恢复与运维接口

#### `restore()`
从存储中恢复任务并重建调度堆。

返回：`number`（恢复任务数）

#### `shutdown()`
停止调度器后台线程。

---

### 2.6 追溯接口（Execution/Audit）

#### `drainExecutionRecords(limit)`
消费式拉取执行记录（读取后移除）。

#### `queryExecutionRecords(limit)`
只读查询执行记录（读取后不移除）。

执行记录字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `string` | 记录 ID |
| `jobId` | `string` | 任务 ID |
| `plannedTimeMs` | `number` | 计划执行时间 |
| `actualTimeMs` | `number` | 实际执行时间 |
| `status` | `string` | 执行状态 |
| `error` | `string \| undefined` | 错误信息 |

#### `drainAuditLogs(limit)`
消费式拉取审计日志（读取后移除）。

#### `queryAuditLogs(limit)`
只读查询审计日志（读取后不移除）。

审计日志字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `timestampMs` | `number` | 事件时间 |
| `event` | `string` | 事件名 |
| `detailJson` | `string` | 详细信息（JSON 字符串） |

## 3. 顶层 Cron 工具函数

### `everyMinutes(n)`
生成“每 N 分钟”表达式。

### `everyHours(n)`
生成“每 N 小时”表达式。

### `dailyAt(hour, minute)`
生成“每天 HH:mm”表达式。

### `weeklyOn(dow, hour, minute)`
生成“每周某天 HH:mm”表达式。

### `monthlyOn(day, hour, minute)`
生成“每月某日 HH:mm”表达式。

## 4. 开发建议

1. 新项目优先使用统一入口 `schedule(input)`。
2. 前端列表刷新使用 `listJobs()`，详情页使用 `getJob(jobId)`。
3. 多系统并发观察请使用 `queryExecutionRecords/queryAuditLogs`，不要使用 `drain*`。
4. 业务退出前调用 `shutdown()`，避免后台线程残留。
