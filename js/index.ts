// Thin TS facade: all scheduling logic lives in Rust core.
import * as native from '../index.js'

export const WhCatTimer = native.WhCatTimer
export const everyMinutes = native.everyMinutes
export const everyHours = native.everyHours
export const dailyAt = native.dailyAt
export const weeklyOn = native.weeklyOn
export const monthlyOn = native.monthlyOn

export type TimerConfigInput = import('../index').TimerConfigInput
export type ScheduleResult = import('../index').ScheduleResult
export type NapiExecutionRecord = import('../index').NapiExecutionRecord
export type NapiAuditRecord = import('../index').NapiAuditRecord
export type NapiJobListItem = import('../index').NapiJobListItem
export type NapiJobSchedule = import('../index').NapiJobSchedule
