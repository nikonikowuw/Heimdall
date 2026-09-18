import { describe, expect, it } from 'vitest'
import {
  denormalizeCosineSimilarity,
  formatCosineSimilarityPercent,
  getCosineSimilarityLevel,
  isCosineThresholdKey,
  normalizeCosineSimilarity,
} from './similarity'

describe('cosine similarity display conversion', () => {
  it('normalizes raw cosine values to the display range', () => {
    expect(normalizeCosineSimilarity(-1)).toBe(0)
    expect(normalizeCosineSimilarity(0)).toBe(0.5)
    expect(normalizeCosineSimilarity(0.75)).toBe(0.875)
    expect(normalizeCosineSimilarity(1)).toBe(1)
  })

  it('converts display values back to raw cosine values', () => {
    expect(denormalizeCosineSimilarity(0.875)).toBeCloseTo(0.75)
    expect(denormalizeCosineSimilarity(0.8)).toBeCloseTo(0.6)
    expect(denormalizeCosineSimilarity(-1)).toBe(-1)
    expect(denormalizeCosineSimilarity(2)).toBe(1)
  })

  it('formats normalized cosine values as percentages', () => {
    expect(formatCosineSimilarityPercent(0.56)).toBe('78.0%')
    expect(formatCosineSimilarityPercent(0.75, 0)).toBe('88%')
    expect(formatCosineSimilarityPercent(0.47047037, 0)).toBe('74%')
    expect(formatCosineSimilarityPercent(0.47047037, 1)).toBe('73.5%')
    expect(formatCosineSimilarityPercent(undefined)).toBe('-')
    expect(formatCosineSimilarityPercent(Number.NaN)).toBe('-')
  })

  it('determines similarity level correctly based on normalized score', () => {
    // 0.75 对应 88%，归一化分值 >= 0.8，判定为 high 翡翠绿
    expect(getCosineSimilarityLevel(0.75)).toBe('high')
    // 0.5 对应 75%，归一化分值 0.75，判定为 medium 暖橙色
    expect(getCosineSimilarityLevel(0.5)).toBe('medium')
    // 0.1 对应 55%，归一化分值 0.55，判定为 low 警示红
    expect(getCosineSimilarityLevel(0.1)).toBe('low')
  })

  it('only treats recognition thresholds as cosine thresholds', () => {
    expect(isCosineThresholdKey('similarity_threshold')).toBe(true)
    expect(isCosineThresholdKey('review_threshold')).toBe(true)
    expect(isCosineThresholdKey('confidence_threshold')).toBe(false)
  })
})
