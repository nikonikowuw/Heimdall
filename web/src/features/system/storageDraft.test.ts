import { describe, expect, it } from 'vitest'
import {
  DEFAULT_RECORDING_RETENTION_DAYS,
  clampStorageDraft,
  percentToRatio,
  ratioToPercent,
  type StorageDraft,
} from './storageDraft'

const BASE: StorageDraft = {
  alarmRetentionDays: 30,
  alarmQuotaMb: 0,
  recognitionRetentionDays: 14,
  recognitionQuotaMb: 0,
  captureRetentionDays: 7,
  captureQuotaMb: 0,
  recordingRetentionDays: 7,
  overwriteMode: 'overwrite',
  autoCleanupEnabled: true,
  minFreeRatio: 0.15,
  targetFreeRatio: 0.25,
  emergencyFreeRatio: 0.08,
  criticalFreeRatio: 0.05,
  batchDeleteSize: 100,
}

function makeConfig(overrides: Partial<StorageDraft> = {}): StorageDraft {
  return { ...BASE, ...overrides }
}

describe('clampStorageDraft 保留天数', () => {
  it('超界收敛到与后端 validate() 一致的 1–365', () => {
    const clamped = clampStorageDraft(
      makeConfig({
        alarmRetentionDays: 0,
        recognitionRetentionDays: 366,
        captureRetentionDays: -3,
        recordingRetentionDays: 9999,
      }),
    )
    expect(clamped.alarmRetentionDays).toBe(1)
    expect(clamped.recognitionRetentionDays).toBe(365)
    expect(clamped.captureRetentionDays).toBe(1)
    expect(clamped.recordingRetentionDays).toBe(365)
  })

  it('区间内取值原样保留（含小数取整）', () => {
    const clamped = clampStorageDraft(makeConfig({ alarmRetentionDays: 30.4 }))
    expect(clamped.alarmRetentionDays).toBe(30)
  })

  it('NaN 回落到下界而不是污染请求体', () => {
    const clamped = clampStorageDraft(makeConfig({ alarmRetentionDays: Number.NaN }))
    expect(clamped.alarmRetentionDays).toBe(1)
  })

  it('字段缺失时回落默认 7，不被夹成 1（旧服务端省略该字段）', () => {
    const legacy: StorageDraft = { ...BASE, recordingRetentionDays: undefined }
    expect(clampStorageDraft(legacy).recordingRetentionDays).toBe(DEFAULT_RECORDING_RETENTION_DAYS)
  })
})

describe('clampStorageDraft 配额', () => {
  it('0 保持 0（不限）而不被折成下界以外的值', () => {
    const clamped = clampStorageDraft(
      makeConfig({ alarmQuotaMb: 0, recognitionQuotaMb: 0, captureQuotaMb: 0 }),
    )
    expect(clamped.alarmQuotaMb).toBe(0)
    expect(clamped.recognitionQuotaMb).toBe(0)
    expect(clamped.captureQuotaMb).toBe(0)
  })

  it('负数收敛到 0，正数无上界', () => {
    const clamped = clampStorageDraft(
      makeConfig({ alarmQuotaMb: -1, recognitionQuotaMb: 10_000_000 }),
    )
    expect(clamped.alarmQuotaMb).toBe(0)
    expect(clamped.recognitionQuotaMb).toBe(10_000_000)
  })
})

describe('clampStorageDraft 水位', () => {
  it('超界收敛到 0–1 比值，不取整', () => {
    const clamped = clampStorageDraft(
      makeConfig({
        minFreeRatio: 1.5,
        targetFreeRatio: 2,
        emergencyFreeRatio: -0.5,
        criticalFreeRatio: 0.05,
      }),
    )
    expect(clamped.minFreeRatio).toBe(1)
    expect(clamped.targetFreeRatio).toBe(1)
    expect(clamped.emergencyFreeRatio).toBe(0)
    expect(clamped.criticalFreeRatio).toBe(0.05)
  })

  it('NaN 回落到 0', () => {
    const clamped = clampStorageDraft(makeConfig({ minFreeRatio: Number.NaN }))
    expect(clamped.minFreeRatio).toBe(0)
  })
})

describe('clampStorageDraft 批删除数', () => {
  it('超界收敛到与后端一致的 10–500', () => {
    expect(clampStorageDraft(makeConfig({ batchDeleteSize: 0 })).batchDeleteSize).toBe(10)
    expect(clampStorageDraft(makeConfig({ batchDeleteSize: 999 })).batchDeleteSize).toBe(500)
    expect(clampStorageDraft(makeConfig({ batchDeleteSize: 100 })).batchDeleteSize).toBe(100)
  })

  it('NaN 回落到 10', () => {
    expect(clampStorageDraft(makeConfig({ batchDeleteSize: Number.NaN })).batchDeleteSize).toBe(10)
  })
})

describe('clampStorageDraft 非数值字段', () => {
  it('枚举与开关原样透传，不被收敛逻辑改写', () => {
    const clamped = clampStorageDraft(
      makeConfig({ overwriteMode: 'stop', autoCleanupEnabled: false }),
    )
    expect(clamped.overwriteMode).toBe('stop')
    expect(clamped.autoCleanupEnabled).toBe(false)
  })
})

describe('水位百分数与比值换算', () => {
  it('比值与百分数往返一致', () => {
    expect(ratioToPercent(0.15)).toBe(15)
    expect(ratioToPercent(0.05)).toBe(5)
    expect(percentToRatio(15)).toBeCloseTo(0.15)
  })

  it('服务端带来的非整百分数按四舍五入显示，不丢可编辑精度', () => {
    // 0.083 → 8%，再写回即为 0.08；这是「界面只承诺整百分数」的有意取舍
    expect(ratioToPercent(0.083)).toBe(8)
    expect(percentToRatio(8)).toBeCloseTo(0.08)
  })
})
