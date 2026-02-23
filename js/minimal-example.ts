import { WhCatTimer, dailyAt } from './index'

// Use sqlite to support cross-restart recovery.
const timer = new WhCatTimer({
  storeKind: 'sqlite',
  sqlitePath: './whcat_timer.db',
  restoreOnStart: true,
  tickMs: 200,
})

const interval = timer.scheduleInterval(
  5_000,
  JSON.stringify({ name: 'heartbeat', source: 'demo' }),
  'demo_interval',
  true,
  0,
)
console.log('interval id =', interval.id)

const cronExpr = dailyAt(3, 30)
const daily = timer.scheduleCron(
  cronExpr,
  'Asia/Shanghai',
  JSON.stringify({ name: 'daily-report' }),
  'demo_daily',
  0,
)
console.log('daily cron id =', daily.id)

setInterval(() => {
  const records = timer.drainExecutionRecords(100)
  for (const rec of records) {
    console.log('[dispatch]', rec.jobId, rec.plannedTimeMs, rec.actualTimeMs, rec.status)
  }
}, 1_000)

// Simulate "restart recovery": call restore after re-instantiating.
setTimeout(() => {
  const restored = timer.restore()
  console.log('restored jobs =', restored)
}, 10_000)
