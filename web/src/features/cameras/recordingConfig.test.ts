import { describe, expect, it } from 'vitest'
import type { CameraRecordingConfig } from '@/types'
import {
  DEFAULT_RECORDING_CONFIG,
  clampRecordingConfig,
  isRecordingConfigDirty,
} from './recordingConfig'

function makeConfig(overrides: Partial<CameraRecordingConfig> = {}): CameraRecordingConfig {
  return { ...DEFAULT_RECORDING_CONFIG, ...overrides }
}

describe('clampRecordingConfig', () => {
  it('超界值收敛到与后端一致的区间', () => {
    const clamped = clampRecordingConfig(
      makeConfig({
        preCaptureSeconds: 0,
        postCaptureSeconds: 999,
        maxFileSeconds: 5,
        retentionDays: 0,
      }),
    )
    expect(clamped.preCaptureSeconds).toBe(5)
    expect(clamped.postCaptureSeconds).toBe(30)
    expect(clamped.maxFileSeconds).toBe(30)
    expect(clamped.retentionDays).toBe(1)
  })

  it('保留区间内的合法值并取整', () => {
    const clamped = clampRecordingConfig(
      makeConfig({
        enabled: true,
        preCaptureSeconds: 12.6,
        postCaptureSeconds: 20,
        maxFileSeconds: 600,
        retentionDays: 30,
      }),
    )
    expect(clamped).toEqual({
      enabled: true,
      preCaptureSeconds: 13,
      postCaptureSeconds: 20,
      maxFileSeconds: 600,
      retentionDays: 30,
    })
  })

  it('NaN 回落到区间下界而不是污染请求体', () => {
    const clamped = clampRecordingConfig(makeConfig({ preCaptureSeconds: Number.NaN }))
    expect(clamped.preCaptureSeconds).toBe(5)
  })
})

describe('isRecordingConfigDirty', () => {
  it('服务端为 null 时与默认值比较', () => {
    expect(isRecordingConfigDirty(makeConfig(), null)).toBe(false)
    expect(isRecordingConfigDirty(makeConfig({ enabled: true }), null)).toBe(true)
  })

  it('逐字段比较服务端快照', () => {
    const server = makeConfig({ enabled: true, retentionDays: 30 })
    expect(isRecordingConfigDirty({ ...server }, server)).toBe(false)
    expect(isRecordingConfigDirty({ ...server, retentionDays: 15 }, server)).toBe(true)
  })
})
