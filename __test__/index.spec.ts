import test from 'ava'

import { WhCatTimer, dailyAt } from '../index'

test('interval + cron + restore api smoke test', (t) => {
  const timer = new WhCatTimer()

  const interval = timer.scheduleInterval(1_000, JSON.stringify({ type: 'interval' }), 'test_interval')
  t.truthy(interval.id)

  const cronExpr = dailyAt(3, 30)
  const cronJob = timer.scheduleCron(cronExpr, 'UTC', JSON.stringify({ type: 'daily' }), 'test_daily')
  t.truthy(cronJob.id)

  const nextRuns = timer.getNextRuns(interval.id, 3)
  t.true(nextRuns.length >= 1)

  const restored = timer.restore()
  t.true(restored >= 2)

  t.true(timer.cancelJob(interval.id))
  t.true(timer.cancelJob(cronJob.id))
  timer.shutdown()
})

test('positional native api works', (t) => {
  const timer = new WhCatTimer()

  const once = timer.scheduleOnce(
    Date.now() + 2_000,
    JSON.stringify({ type: 'once', from: 'native' }),
    'native_once',
  )
  t.truthy(once.id)

  const interval = timer.scheduleInterval(
    1_000,
    JSON.stringify({ type: 'interval', from: 'native' }),
    'native_interval',
    true,
    3,
  )
  t.truthy(interval.id)

  const cron = timer.scheduleCron(
    dailyAt(1, 2),
    'UTC',
    JSON.stringify({ type: 'cron', from: 'native' }),
    'native_cron',
    2,
  )
  t.truthy(cron.id)

  t.true(timer.cancelJob(interval.id))
  t.true(timer.cancelJob(once.id))
  t.true(timer.cancelJob(cron.id))
  timer.shutdown()
})

test('invalid negative count is rejected', (t) => {
  const timer = new WhCatTimer()
  const err = t.throws(
    () =>
      timer.scheduleInterval(
        1_000,
        JSON.stringify({ invalid: true }),
        'bad_count',
        true,
        -1,
      ),
    { instanceOf: Error },
  )
  t.regex(err.message, /count must be a non-negative integer/)
  timer.shutdown()
})

test('audit logs are observable for monitoring', (t) => {
  const timer = new WhCatTimer({ restoreOnStart: true, maxExecutionRecords: 8 })
  void timer.restore()
  const logs = timer.drainAuditLogs(20)
  t.true(Array.isArray(logs))
  t.true(logs.some((log) => log.event === 'restore_completed'))
  timer.shutdown()
})

test('list jobs returns normalized snapshot for frontend updates', (t) => {
  const timer = new WhCatTimer()
  const interval = timer.scheduleInterval(
    2_000,
    JSON.stringify({ type: 'list-check' }),
    'list_job_interval',
    true,
    2,
  )
  t.truthy(interval.id)
  const rows = timer.listJobs()
  const job = rows.find((row) => row.jobId === interval.id)
  t.truthy(job)
  t.is(job?.jobType, 'interval')
  t.truthy(typeof job?.enabled === 'boolean')
  t.truthy(typeof job?.runCount === 'number')
  t.true(job?.nextRunMs !== undefined)
  t.true(timer.cancelJob(interval.id))
  timer.shutdown()
})

test('get job and readonly query interfaces are available', (t) => {
  const timer = new WhCatTimer()
  const job = timer.scheduleInterval(
    1_000,
    JSON.stringify({ type: 'readonly-query' }),
    'readonly_query_job',
    true,
    1,
  )
  const detail = timer.getJob(job.id)
  t.is(detail.jobId, job.id)
  t.is(detail.jobType, 'interval')
  t.truthy(typeof detail.runCount === 'number')

  const execRows = timer.queryExecutionRecords(10)
  t.true(Array.isArray(execRows))
  const auditRows = timer.queryAuditLogs(10)
  t.true(Array.isArray(auditRows))
  timer.shutdown()
})

test('unified schedule api works', (t) => {
  const timer = new WhCatTimer()
  const cron = timer.schedule({
    type: 'cron',
    cronExpr: dailyAt(5, 6),
    tz: 'UTC',
    payloadJson: JSON.stringify({ from: 'unified' }),
    count: 2,
    jobId: 'unified_cron',
  })
  t.truthy(cron.id)
  const found = timer.listJobs().find((row) => row.jobId === cron.id)
  t.truthy(found)
  t.true(timer.cancelJob(cron.id))
  timer.shutdown()
})
