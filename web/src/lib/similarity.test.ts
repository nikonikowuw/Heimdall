import { describe, expect, it } from 'vitest'
import {
  denormalizeCosineSimilarity,
  formatCosineSimilarityPercent,
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
    expect(formatCosineSimilarityPercent(undefined)).toBe('-')
    expect(formatCosineSimilarityPercent(Number.NaN)).toBe('-')
  })

  it('only treats recognition thresholds as cosine thresholds', () => {
    expect(isCosineThresholdKey('similarity_threshold')).toBe(true)
    expect(isCosineThresholdKey('review_threshold')).toBe(true)
    expect(isCosineThresholdKey('confidence_threshold')).toBe(false)
  })
})
