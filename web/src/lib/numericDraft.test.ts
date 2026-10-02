import { describe, expect, it } from 'vitest'
import {
  clampNumericParam,
  formatNumericDraft,
  isFiniteNumber,
  parseNumericDraft,
} from './numericDraft'

describe('parseNumericDraft 键盘中间态', () => {
  it('保留空串与符号/小数点等未完成输入，不当作 0 或清空意图', () => {
    for (const draft of ['', '   ', '-', '+', '.', '-.', '+.']) {
      expect(parseNumericDraft(draft, 'number')).toBeNull()
      expect(parseNumericDraft(draft, 'integer')).toBeNull()
    }
  })

  it('小数草稿在 number 语义下可解析', () => {
    expect(parseNumericDraft('0.65', 'number')).toBe(0.65)
    expect(parseNumericDraft('-0.5', 'number')).toBe(-0.5)
  })

  it('整型语义拒绝带小数点的草稿，避免 12.5 被当成 13 提交', () => {
    expect(parseNumericDraft('', 'integer')).toBeNull()
    expect(parseNumericDraft('12.5', 'integer')).toBeNull()
    expect(parseNumericDraft('24', 'integer')).toBe(24)
    expect(parseNumericDraft('-3', 'integer')).toBe(-3)
  })

  it('不可解析内容返回 null 而不是 NaN', () => {
    expect(parseNumericDraft('abc', 'number')).toBeNull()
    expect(parseNumericDraft('1e', 'number')).toBeNull()
    expect(parseNumericDraft('--', 'integer')).toBeNull()
    expect(parseNumericDraft('Infinity', 'number')).toBeNull()
  })

  it('接受两侧空白（粘贴或输入法带入）', () => {
    expect(parseNumericDraft(' 42 ', 'integer')).toBe(42)
  })
})

describe('clampNumericParam 收敛', () => {
  it('可解析但超界时收敛到边界', () => {
    expect(clampNumericParam(0, 1, 32)).toBe(1)
    expect(clampNumericParam(99999, 30, 3600)).toBe(3600)
    expect(clampNumericParam(150, 0, 100)).toBe(100)
  })

  it('缺省边界表示该侧不设限，不误收敛', () => {
    expect(clampNumericParam(30, 1, undefined)).toBe(30)
    expect(clampNumericParam(-5, undefined, 100)).toBe(-5)
    expect(clampNumericParam(1e9, undefined, undefined)).toBe(1e9)
  })

  it('区间内的值原样通过（含浮点）', () => {
    expect(clampNumericParam(0.4, 0, 1)).toBe(0.4)
    expect(clampNumericParam(7, 1, 365)).toBe(7)
  })

  it('非有限数值（NaN / Infinity）安全收敛到边界', () => {
    expect(clampNumericParam(Number.NaN, 1, 32)).toBe(1)
    expect(clampNumericParam(Number.NaN, 0, 100)).toBe(0)
    expect(clampNumericParam(Number.NaN, undefined, 100)).toBe(100)
    expect(clampNumericParam(Number.NaN, undefined, undefined)).toBe(0)
    expect(clampNumericParam(Number.POSITIVE_INFINITY, 1, 32)).toBe(32)
    expect(clampNumericParam(Number.NEGATIVE_INFINITY, 1, 32)).toBe(1)
  })
})

describe('formatNumericDraft 回写', () => {
  it('整型字段取整后再格式化，不把 12.5 回写成小数', () => {
    expect(formatNumericDraft(30, 'integer')).toBe('30')
    expect(formatNumericDraft(12.5, 'integer')).toBe('13')
  })

  it('number 字段保留原值', () => {
    expect(formatNumericDraft(0.65, 'number')).toBe('0.65')
    expect(formatNumericDraft(0, 'number')).toBe('0')
  })
})

describe('isFiniteNumber', () => {
  it('只接受有限数值', () => {
    expect(isFiniteNumber(0)).toBe(true)
    expect(isFiniteNumber(Number.NaN)).toBe(false)
    expect(isFiniteNumber(Number.POSITIVE_INFINITY)).toBe(false)
    expect(isFiniteNumber('1')).toBe(false)
    expect(isFiniteNumber(null)).toBe(false)
    expect(isFiniteNumber(undefined)).toBe(false)
  })
})
