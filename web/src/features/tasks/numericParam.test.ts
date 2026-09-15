import { describe, expect, it } from 'vitest'
import {
  clampNumericParam,
  formatNumericDraft,
  getNumericParamConfig,
  parseNumericDraft,
} from './numericParam'

describe('numeric task parameter editing', () => {
  it('keeps an omitted maximum unbounded instead of defaulting it to one', () => {
    expect(getNumericParamConfig({ type: 'integer', minimum: 1, default: 30 })).toEqual({
      type: 'integer',
      minimum: 1,
      maximum: undefined,
      defaultValue: 30,
    })
    expect(clampNumericParam(30, 1, undefined)).toBe(30)
  })

  it('allows an empty draft while a user replaces an existing value', () => {
    expect(parseNumericDraft('', 'number')).toBeNull()
    expect(parseNumericDraft('.', 'number')).toBeNull()
    expect(parseNumericDraft('0.65', 'number')).toBe(0.65)
  })

  it('does not turn a partial integer draft into a replacement value', () => {
    expect(parseNumericDraft('', 'integer')).toBeNull()
    expect(parseNumericDraft('12.5', 'integer')).toBeNull()
    expect(parseNumericDraft('24', 'integer')).toBe(24)
  })

  it('clamps only the committed value and keeps integer formatting stable', () => {
    expect(clampNumericParam(0, 1, undefined)).toBe(1)
    expect(clampNumericParam(2, 0, 1)).toBe(1)
    expect(formatNumericDraft(30, 'integer')).toBe('30')
  })
})
