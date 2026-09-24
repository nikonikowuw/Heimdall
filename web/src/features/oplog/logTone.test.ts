import { describe, expect, it } from 'vitest'
import {
  classifyHttpStatus,
  classifyLatency,
  getToneClasses,
  latencyTone,
  levelTone,
  methodTone,
  statusTone,
  type LogTone,
} from './logTone'

const ALL_TONES: readonly LogTone[] = ['neutral', 'accent', 'success', 'warning', 'danger']

describe('classifyHttpStatus', () => {
  it('maps each HTTP class to its boundary values', () => {
    expect(classifyHttpStatus(200)).toBe('success')
    expect(classifyHttpStatus(204)).toBe('success')
    expect(classifyHttpStatus(299)).toBe('success')
    expect(classifyHttpStatus(400)).toBe('clientError')
    expect(classifyHttpStatus(499)).toBe('clientError')
    expect(classifyHttpStatus(500)).toBe('serverError')
    expect(classifyHttpStatus(503)).toBe('serverError')
  })

  it('buckets statuses outside 2xx/4xx/5xx as other', () => {
    expect(classifyHttpStatus(0)).toBe('other')
    expect(classifyHttpStatus(100)).toBe('other')
    expect(classifyHttpStatus(301)).toBe('other')
    expect(classifyHttpStatus(399)).toBe('other')
  })

  it('keeps the 4xx/5xx boundary out of the success bucket', () => {
    expect(statusTone(399)).toBe(statusTone(301))
    expect(statusTone(400)).not.toBe(statusTone(399))
  })
})

describe('classifyLatency', () => {
  it('splits tiers on the documented 100ms / 500ms boundaries', () => {
    expect(classifyLatency(0)).toBe('fast')
    expect(classifyLatency(99)).toBe('fast')
    expect(classifyLatency(100)).toBe('normal')
    expect(classifyLatency(499)).toBe('normal')
    expect(classifyLatency(500)).toBe('slow')
    expect(latencyTone(500)).toBe('danger')
  })
})

describe('methodTone', () => {
  it('is case insensitive and falls back to neutral', () => {
    expect(methodTone('get')).toBe(methodTone('GET'))
    expect(methodTone('delete')).toBe('danger')
    expect(methodTone('PATCH')).toBe(methodTone('PUT'))
    expect(methodTone('TRACE')).toBe('neutral')
  })
})

describe('levelTone', () => {
  it('covers every level of the server contract', () => {
    expect(levelTone('error')).toBe('danger')
    expect(levelTone('warn')).toBe('warning')
    expect(levelTone('info')).toBe('accent')
  })
})

describe('getToneClasses', () => {
  it('uses the fixed solid danger token for white-on-red segments', () => {
    const danger = getToneClasses('danger')

    expect(danger.solid).toContain('var(--status-danger-solid)')
    expect(danger.solid).toContain('text-white')
    expect(danger.text).toContain('var(--status-danger)')
  })

  it('exposes a complete class set for every tone', () => {
    for (const tone of ALL_TONES) {
      const classes = getToneClasses(tone)
      for (const value of [classes.badge, classes.chip, classes.dot, classes.solid, classes.text]) {
        expect(value.length).toBeGreaterThan(0)
      }
    }
  })
})
