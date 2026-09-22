import { describe, expect, it } from 'vitest'
import {
  formatCosineSimilarityPercent,
  getCosineSimilarityLevel,
  isCosineThresholdKey,
  percentToScore,
  scoreToPercent,
} from './similarity'

describe('cosine similarity display conversion', () => {
  it('maps calibration-domain thresholds to percentages without re-applying the calibration curve', () => {
    // 阈值参数与底库检索输出同处标定置信分域，控制台只做 ×100 线性缩放：
    // 宿主 capture_service 会把 params_json.similarity_threshold 当标定置信分直接消费。
    expect(scoreToPercent(0.75)).toBe(75)
    expect(scoreToPercent(0.78)).toBe(78)
    expect(scoreToPercent(0.5)).toBe(50)
    expect(scoreToPercent(1)).toBe(100)
    expect(scoreToPercent(0)).toBe(0)

    // 回归锁：旧实现把展示值 75% 经标定曲线反解成原始余弦 0.456，
    // 宿主按标定域消费后实际放行线降到 45.6%，确认门槛静默丢失近 30 个点。
    expect(scoreToPercent(0.75)).not.toBeCloseTo(45.6)
    expect(scoreToPercent(0.75)).not.toBeCloseTo(0.456)
  })

  it('round-trips threshold percentages back to the identical calibration score', () => {
    // 界面设 75% 必须原样下发 0.75，不得落到旧标定反解值 0.45599999999999996。
    expect(percentToScore(75)).toBe(0.75)

    for (const score of [0.75, 0.8, 0.6, 0.68, 0.95, 0.456]) {
      expect(percentToScore(scoreToPercent(score))).toBe(score)
    }
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
    expect(getCosineSimilarityLevel(0.5)).toBe('low')
  })

  it('only treats recognition thresholds as cosine thresholds', () => {
    expect(isCosineThresholdKey('similarity_threshold')).toBe(true)
    expect(isCosineThresholdKey('review_threshold')).toBe(true)
    expect(isCosineThresholdKey('confidence_threshold')).toBe(false)
  })
})
