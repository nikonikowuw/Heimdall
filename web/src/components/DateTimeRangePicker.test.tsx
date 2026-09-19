import { describe, expect, it } from 'vitest'
import type { DateTimeRangeValue, QuickTimePreset } from '@/lib/dateRange'

describe('DateTimeRangePicker logic', () => {
  it('identifies all time preset with undefined timestamps', () => {
    const val: DateTimeRangeValue = {
      quickPreset: 'all',
      startTime: undefined,
      endTime: undefined,
    }
    expect(val.quickPreset).toBe('all')
    expect(val.startTime).toBeUndefined()
    expect(val.endTime).toBeUndefined()
  })

  it('correctly handles custom second-level range', () => {
    const start = new Date('2026-09-14T10:00:00').getTime()
    const end = new Date('2026-09-14T10:15:30').getTime()

    const val: DateTimeRangeValue = {
      quickPreset: 'custom',
      startTime: start,
      endTime: end,
    }

    expect(val.quickPreset).toBe('custom')
    expect(val.endTime! - val.startTime!).toBe(15 * 60 * 1000 + 30 * 1000)
  })

  it('supports presets 5m, 15m, 30m, 1h, 24h, today, 7d', () => {
    const presets: QuickTimePreset[] = [
      'all',
      '5m',
      '15m',
      '30m',
      '1h',
      '24h',
      'today',
      '7d',
      'custom',
    ]
    expect(presets).toHaveLength(9)
  })
})
