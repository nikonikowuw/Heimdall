import { describe, expect, it } from 'vitest'
import { isMemberOf, narrowSelectValue } from './unionNarrowing'

const STATUS_FILTERS = ['all', 'success', 'failed'] as const
const EVENT_FILTERS = ['camera_offline', 'camera_online'] as const

describe('isMemberOf', () => {
  it('accepts declared values and rejects everything else', () => {
    expect(isMemberOf(STATUS_FILTERS, 'failed')).toBe(true)
    expect(isMemberOf(STATUS_FILTERS, 'clientError')).toBe(false)
    expect(isMemberOf(STATUS_FILTERS, '')).toBe(false)
  })

  it('is case sensitive, matching the exact wire values', () => {
    expect(isMemberOf(EVENT_FILTERS, 'camera_offline')).toBe(true)
    expect(isMemberOf(EVENT_FILTERS, 'CAMERA_OFFLINE')).toBe(false)
  })
})

describe('narrowSelectValue', () => {
  const OPTIONS = [
    { value: 'all', label: '全部' },
    { value: 'roi', label: '区域入侵' },
  ] as const

  it('returns the matching option value for a declared key', () => {
    expect(narrowSelectValue<string>('roi', OPTIONS)).toBe('roi')
  })

  it('returns undefined for a key absent from the option table', () => {
    // 这是替代 `as` 断言的核心保证：未声明的值必须被拦下而非透传
    expect(narrowSelectValue<string>('line', OPTIONS)).toBeUndefined()
    expect(narrowSelectValue<string>('', OPTIONS)).toBeUndefined()
  })

  it('accepts the all-option value when provided', () => {
    const all = { value: '', label: '不限' }
    expect(narrowSelectValue<string>('', OPTIONS, all)).toBe('')
  })

  it('does not treat the all-option as a wildcard for other keys', () => {
    const all = { value: '', label: '不限' }
    expect(narrowSelectValue<string>('line', OPTIONS, all)).toBeUndefined()
  })

  it('only needs a value shape, so richer option objects are accepted', () => {
    const rich = [{ value: 'a', label: 'A', disabled: true, extra: 1 }] as const
    expect(narrowSelectValue<string>('a', rich)).toBe('a')
  })
})
