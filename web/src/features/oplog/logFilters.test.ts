import { describe, expect, it } from 'vitest'
import {
  EVENT_FILTERS,
  isMemberOf,
  LEVEL_FILTERS,
  MODULE_FILTERS,
  STATUS_FILTERS,
  TARGET_FILTERS,
} from './logFilters'

describe('isMemberOf', () => {
  it('accepts declared values and rejects everything else', () => {
    expect(isMemberOf(STATUS_FILTERS, 'failed')).toBe(true)
    expect(isMemberOf(STATUS_FILTERS, 'clientError')).toBe(false)
    expect(isMemberOf(MODULE_FILTERS, 'all')).toBe(true)
    expect(isMemberOf(MODULE_FILTERS, 'unknown-module')).toBe(false)
    expect(isMemberOf(MODULE_FILTERS, '')).toBe(false)
  })

  it('is case sensitive, matching the exact wire values', () => {
    expect(isMemberOf(EVENT_FILTERS, 'camera_offline')).toBe(true)
    expect(isMemberOf(EVENT_FILTERS, 'CAMERA_OFFLINE')).toBe(false)
  })
})

describe('filter vocabularies', () => {
  it('offers an explicit unfiltered option in every vocabulary', () => {
    expect(LEVEL_FILTERS).toContain('all')
    expect(TARGET_FILTERS).toContain('all')
    expect(EVENT_FILTERS).toContain('all')
  })

  it('keeps the level vocabulary aligned with OpEvent::level', () => {
    expect(LEVEL_FILTERS.filter((level) => level !== 'all')).toEqual(['info', 'warn', 'error'])
  })

  it('keeps the target vocabulary aligned with OpEvent::target', () => {
    expect(TARGET_FILTERS.filter((target) => target !== 'all')).toEqual([
      'system',
      'media',
      'pipeline',
      'rule',
      'infer',
      'hardware',
      'storage',
    ])
  })

  it('declares every OpEvent tag exactly once, with no duplicates in the event list', () => {
    const tags = EVENT_FILTERS.filter((tag) => tag !== 'all')
    expect(tags).toHaveLength(18)
    expect(new Set(tags).size).toBe(tags.length)
    expect(tags).toContain('service_started')
    expect(tags).toContain('task_stopped')
    expect(tags).toContain('algo_load_failed')
    expect(tags).toContain('storage_eviction')
  })
})
