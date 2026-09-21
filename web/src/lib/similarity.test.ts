import { describe, expect, it } from 'vitest'
import {
  denormalizeCosineSimilarity,
  formatCosineSimilarityPercent,
  getCosineSimilarityLevel,
  isCosineThresholdKey,
  normalizeCosineSimilarity,
} from './similarity'

describe('cosine similarity display conversion', () => {
  it('normalizes raw cosine values to the display range using Megvii-style multi-anchor calibration', () => {
    // 理论下限：-1.0 -> 0%
    expect(normalizeCosineSimilarity(-1.5)).toBe(0)
    expect(normalizeCosineSimilarity(-1.0)).toBe(0)
    expect(normalizeCosineSimilarity(-0.5)).toBeCloseTo(0.25)

    // 正交随机基准点 (余弦 0.00 -> 50.0%)
    expect(normalizeCosineSimilarity(0.0)).toBeCloseTo(0.5)

    // 旷视实测底库负样本候选分布：
    // 白俊聪 (0.026 -> 51.3%, 0.050 -> 52.5%)
    expect(normalizeCosineSimilarity(0.026)).toBeCloseTo(0.513)
    expect(normalizeCosineSimilarity(0.05)).toBeCloseTo(0.525)

    // 许丽娜 (0.052 -> 52.6%)
    expect(normalizeCosineSimilarity(0.052)).toBeCloseTo(0.526)

    // 林青 (0.102 -> 55.1%)
    expect(normalizeCosineSimilarity(0.102)).toBeCloseTo(0.551)

    // 疑似复核门限锚点 (余弦 0.40 -> 68.0%，对标旷视 1e-4)
    expect(normalizeCosineSimilarity(0.4)).toBeCloseTo(0.68)

    // 确认放行门限锚点 (similarity_threshold: 0.48 -> 78.0%，对标旷视 1e-5 金融级)
    expect(normalizeCosineSimilarity(0.48)).toBeCloseTo(0.78)

    // 远景单帧抓拍 (0.504 -> 80.5%)
    expect(normalizeCosineSimilarity(0.504)).toBeCloseTo(0.805)

    // 时域融合近景高质 (0.584 -> 88.5%)
    expect(normalizeCosineSimilarity(0.584)).toBeCloseTo(0.885)

    // 理论上限：1.00 -> 100%
    expect(normalizeCosineSimilarity(1.0)).toBe(1)
    expect(normalizeCosineSimilarity(1.5)).toBe(1)
  })

  it('converts display values back to raw cosine values (exact inversion)', () => {
    expect(denormalizeCosineSimilarity(0)).toBe(-1.0)
    expect(denormalizeCosineSimilarity(0.25)).toBeCloseTo(-0.5)
    expect(denormalizeCosineSimilarity(0.5)).toBeCloseTo(0.0)
    expect(denormalizeCosineSimilarity(0.513)).toBeCloseTo(0.026)
    expect(denormalizeCosineSimilarity(0.526)).toBeCloseTo(0.052)
    expect(denormalizeCosineSimilarity(0.55)).toBeCloseTo(0.1)
    expect(denormalizeCosineSimilarity(0.68)).toBeCloseTo(0.4)
    expect(denormalizeCosineSimilarity(0.78)).toBeCloseTo(0.48)
    expect(denormalizeCosineSimilarity(0.884)).toBeCloseTo(0.58)
    expect(denormalizeCosineSimilarity(1.0)).toBe(1.0)
    expect(denormalizeCosineSimilarity(-0.1)).toBe(-1.0)
    expect(denormalizeCosineSimilarity(1.2)).toBe(1.0)
  })

  it('formats normalized cosine values as percentages matching Megvii precision', () => {
    // 直接消费算法包与后端输出的标定分值：
    expect(formatCosineSimilarityPercent(0.513, 1)).toBe('51.3%')
    expect(formatCosineSimilarityPercent(0.525, 1)).toBe('52.5%')
    expect(formatCosineSimilarityPercent(0.526, 1)).toBe('52.6%')
    expect(formatCosineSimilarityPercent(0.551, 1)).toBe('55.1%')

    // 业务门限与同人命中分：
    expect(formatCosineSimilarityPercent(0.68, 0)).toBe('68%')
    expect(formatCosineSimilarityPercent(0.78, 0)).toBe('78%')
    expect(formatCosineSimilarityPercent(0.805, 1)).toBe('80.5%')
    expect(formatCosineSimilarityPercent(0.885, 1)).toBe('88.5%')

    expect(formatCosineSimilarityPercent(undefined)).toBe('-')
    expect(formatCosineSimilarityPercent(Number.NaN)).toBe('-')
  })

  it('determines similarity level correctly based on normalized score', () => {
    // 标定分值 >= 0.78，判定为 high 翡翠绿 (确认放行)
    expect(getCosineSimilarityLevel(0.78)).toBe('high')
    expect(getCosineSimilarityLevel(0.885)).toBe('high')
    expect(getCosineSimilarityLevel(0.95)).toBe('high')

    // 标定分值 [0.60, 0.78)，判定为 medium 暖橙色 (疑似复核)
    expect(getCosineSimilarityLevel(0.615)).toBe('medium')
    expect(getCosineSimilarityLevel(0.68)).toBe('medium')

    // 50~55 分的负样本底噪候选人判定为 low (未命中)
    expect(getCosineSimilarityLevel(0.551)).toBe('low') // 林青 55.1%
    expect(getCosineSimilarityLevel(0.526)).toBe('low') // 许丽娜 52.6%
    expect(getCosineSimilarityLevel(0.50)).toBe('low')
  })

  it('only treats recognition thresholds as cosine thresholds', () => {
    expect(isCosineThresholdKey('similarity_threshold')).toBe(true)
    expect(isCosineThresholdKey('review_threshold')).toBe(true)
    expect(isCosineThresholdKey('confidence_threshold')).toBe(false)
  })
})
