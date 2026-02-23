import { Bench } from 'tinybench'

import { WhCatTimer } from '../index.js'

const timer = new WhCatTimer()

const b = new Bench()

b.add('Native scheduleInterval', () => {
  const id = `bench_${Date.now()}_${Math.random()}`
  timer.scheduleInterval(60_000, '{"bench":true}', id)
  timer.cancelJob(id)
})

b.add('JavaScript noop', () => {
  return 1
})

void b.run().then(() => {
  console.table(b.table())
  timer.shutdown()
})
