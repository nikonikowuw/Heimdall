import { describe, expect, it } from 'vitest'
import {
  denormalizeCosineSimilarity,
  formatCosineSimilarityPercent,
  getCosineSimilarityLevel,
  isCosineThresholdKey,
  normalizeCosineSimilarity,
} from './similarity'

describe('cosine similarity display conversion', () => {
  it('normalizes raw cosine values to the display range using three-anchor calibration', () => {
    // 负样本/陌生人物理荒漠区 (<= 0.20) 归零
    expect(normalizeCosineSimilarity(-1)).toBe(0)
    expect(normalizeCosineSimilarity(0)).toBe(0)
    expect(normalizeCosineSimilarity(0.2)).toBe(0)

    // 负样本过渡区：宋柱良 0.25 -> 10%
    expect(normalizeCosineSimilarity(0.25)).toBeCloseTo(0.1)

    // 疑似复核门限锚点 (review_threshold: 0.50 -> 60%)
    expect(normalizeCosineSimilarity(0.5)).toBeCloseTo(0.6)

    // 确认放行门限锚点 (similarity_threshold: 0.60 -> 80%)
    expect(normalizeCosineSimilarity(0.6)).toBeCloseTo(0.8)

    // 高置信正样本：0.70 -> 85%
    expect(normalizeCosineSimilarity(0.7)).toBeCloseTo(0.85)

    // 理论上限：1.00 -> 100%
    expect(normalizeCosineSimilarity(1.0)).toBe(1)
    expect(normalizeCosineSimilarity(1.5)).toBe(1)
  })

  it('converts display values back to raw cosine values (exact inversion)', () => {
    expect(denormalizeCosineSimilarity(0)).toBe(0.2)
    expect(denormalizeCosineSimilarity(0.1)).toBeCloseTo(0.25)
    expect(denormalizeCosineSimilarity(0.6)).toBeCloseTo(0.5)
    expect(denormalizeCosineSimilarity(0.8)).toBeCloseTo(0.6)
    expect(denormalizeCosineSimilarity(0.85)).toBeCloseTo(0.7)
    expect(denormalizeCosineSimilarity(1.0)).toBe(1.0)
    expect(denormalizeCosineSimilarity(-0.1)).toBe(0.2)
    expect(denormalizeCosineSimilarity(1.2)).toBe(1.0)
  })

  it('formats normalized cosine values as percentages', () => {
    // 0.50 (复核线) -> 60%
    expect(formatCosineSimilarityPercent(0.5, 0)).toBe('60%')
    // 0.60 (放行线) -> 80%
    expect(formatCosineSimilarityPercent(0.6, 0)).toBe('80%')
    // 0.6235 (张科实测融合分) -> 81.2%
    expect(formatCosineSimilarityPercent(0.6235, 1)).toBe('81.2%')
    // 0.25 (宋柱良干扰分) -> 10%
    expect(formatCosineSimilarityPercent(0.25, 0)).toBe('10%')
    // 0.10 (无关陌生人) -> 0%
    expect(formatCosineSimilarityPercent(0.1, 0)).toBe('0%')

    expect(formatCosineSimilarityPercent(undefined)).toBe('-')
    expect(formatCosineSimilarityPercent(Number.NaN)).toBe('-')
  })

  it('determines similarity level correctly based on normalized score', () => {
    // 0.60 对应 80%，归一化分值 >= 0.8，判定为 high 翡翠绿 (确认放行)
    expect(getCosineSimilarityLevel(0.6)).toBe('high')
    expect(getCosineSimilarityLevel(0.65)).toBe('high')

    // 0.50 对应 60%，归一化分值 [0.6, 0.8)，判定为 medium 暖橙色 (疑似复核)
    expect(getCosineSimilarityLevel(0.5)).toBe('medium')
    expect(getCosineSimilarityLevel(0.55)).toBe('medium')

    // 0.49 对应 58%，归一化分值 < 0.6，判定为 low 警示红 (未命中)
    expect(getCosineSimilarityLevel(0.49)).toBe('low')
    expect(getCosineSimilarityLevel(0.25)).toBe('low')
    expect(getCosineSimilarityLevel(0.0)).toBe('low')
  })

  it('only treats recognition thresholds as cosine thresholds', () => {
    expect(isCosineThresholdKey('similarity_threshold')).toBe(true)
    expect(isCosineThresholdKey('review_threshold')).toBe(true)
    expect(isCosineThresholdKey('confidence_threshold')).toBe(false)
  })
})
