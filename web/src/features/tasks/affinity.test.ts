import { describe, expect, it } from 'vitest'
import { formatAffinityBadge } from './affinity'

describe('formatAffinityBadge', () => {
  it('formats undefined as Auto·Spread', () => {
    expect(formatAffinityBadge(undefined)).toBe('Auto·Spread')
    expect(formatAffinityBadge(null)).toBe('Auto·Spread')
  })

  it('formats auto spread explicitly', () => {
    expect(formatAffinityBadge({ mode: 'auto', policy: 'spread' })).toBe('Auto·Spread')
  })

  it('formats auto pack correctly', () => {
    expect(formatAffinityBadge({ mode: 'auto', policy: 'pack' })).toBe('Auto·Pack')
  })

  it('formats manual core index correctly', () => {
    expect(formatAffinityBadge({ mode: 'manual', deviceId: 'rknn-npu0', coreIndex: 0 })).toBe(
      'Core 0',
    )
    expect(formatAffinityBadge({ mode: 'manual', deviceId: 'rknn-npu0', coreIndex: 1 })).toBe(
      'Core 1',
    )
    expect(formatAffinityBadge({ mode: 'manual', deviceId: 'rknn-npu0', coreIndex: 2 })).toBe(
      'Core 2',
    )
  })
})
