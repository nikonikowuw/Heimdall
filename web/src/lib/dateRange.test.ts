import { describe, expect, it } from 'vitest'
import { resolveEffectiveTimeRange, resolvePresetTimestamps } from './dateRange'

describe('resolveEffectiveTimeRange', () => {
  const fixedNow = new Date('2026-03-20T14:30:00.000Z').getTime()

  it('resolves "all" preset to undefined boundaries', () => {
    const res = resolveEffectiveTimeRange({ quickPreset: 'all' }, fixedNow)
    expect(res.startTime).toBeUndefined()
    expect(res.endTime).toBeUndefined()
  })

  it('preserves exact timestamps for "custom" preset', () => {
    const res = resolveEffectiveTimeRange(
      { quickPreset: 'custom', startTime: 1000, endTime: 2000 },
      fixedNow,
    )
    expect(res.startTime).toBe(1000)
    expect(res.endTime).toBe(2000)
  })

  it('resolves "today" preset starting at 00:00:00 and without upper bound limitation', () => {
    const res = resolveEffectiveTimeRange({ quickPreset: 'today' }, fixedNow)
    const expectedStart = new Date(fixedNow)
    expectedStart.setHours(0, 0, 0, 0)
    expect(res.startTime).toBe(expectedStart.getTime())
    expect(res.endTime).toBeUndefined()
  })

  it('resolves relative duration presets dynamically based on now', () => {
    const res5m = resolveEffectiveTimeRange({ quickPreset: '5m' }, fixedNow)
    expect(res5m.startTime).toBe(fixedNow - 5 * 60_000)
    expect(res5m.endTime).toBe(fixedNow)

    const res1h = resolveEffectiveTimeRange({ quickPreset: '1h' }, fixedNow)
    expect(res1h.startTime).toBe(fixedNow - 3600_000)
    expect(res1h.endTime).toBe(fixedNow)
  })
})

describe('resolvePresetTimestamps', () => {
  const fixedNow = new Date('2026-03-20T14:30:00.000Z').getTime()

  // 选择器回填 draft 值必须与解析器取到同一边界，否则同一预设会显示与过滤出不同的区间。
  it('produces boundaries identical to resolveEffectiveTimeRange for every preset', () => {
    const presets = ['all', '5m', '15m', '30m', '1h', '24h', '7d', 'today'] as const
    for (const preset of presets) {
      expect(resolvePresetTimestamps(preset, fixedNow)).toEqual(
        resolveEffectiveTimeRange({ quickPreset: preset }, fixedNow),
      )
    }
  })

  it('returns undefined boundaries for "custom" which carries no preset window', () => {
    expect(resolvePresetTimestamps('custom', fixedNow)).toEqual({
      startTime: undefined,
      endTime: undefined,
    })
  })
})
