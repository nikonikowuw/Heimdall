import { describe, expect, it } from 'vitest'
import {
  calculateFittedImageRect,
  formatTimestamp,
  getRuleTypeLabel,
  parseBBoxCoords,
  parseTargetBBoxes,
  resolveEffectiveTimeRange,
} from './utils'

describe('alarms utils', () => {
  describe('formatTimestamp', () => {
    it('returns "-" for empty values', () => {
      expect(formatTimestamp()).toBe('-')
      expect(formatTimestamp(null)).toBe('-')
      expect(formatTimestamp('')).toBe('-')
    })

    it('formats numeric timestamps', () => {
      const ts = 1700000000000
      expect(formatTimestamp(ts)).toBe(new Date(ts).toLocaleString())
    })
  })

  describe('parseBBoxCoords', () => {
    it('returns null for empty or invalid json', () => {
      expect(parseBBoxCoords('')).toBeNull()
      expect(parseBBoxCoords('invalid-json')).toBeNull()
      expect(parseBBoxCoords('[]')).toBeNull()
    })

    it('parses array format [x1, y1, x2, y2]', () => {
      const res = parseBBoxCoords('[0.1, 0.2, 0.8, 0.9]')
      expect(res).toEqual({ x1: 0.1, y1: 0.2, x2: 0.8, y2: 0.9 })
    })

    it('parses object format { x1, y1, x2, y2 }', () => {
      const res = parseBBoxCoords('{"x1": 0.1, "y1": 0.2, "x2": 0.8, "y2": 0.9}')
      expect(res).toEqual({ x1: 0.1, y1: 0.2, x2: 0.8, y2: 0.9 })
    })

    it('parses composite format extracting body for backward compatibility', () => {
      const res = parseBBoxCoords(
        '{"body": [0.1, 0.2, 0.4, 0.6], "face": {"bbox": [0.15, 0.22, 0.25, 0.35], "qualityScore": 0.88}}',
      )
      expect(res).toEqual({ x1: 0.1, y1: 0.2, x2: 0.4, y2: 0.6 })
    })
  })

  describe('parseTargetBBoxes', () => {
    it('returns null for empty or invalid json', () => {
      expect(parseTargetBBoxes('')).toBeNull()
      expect(parseTargetBBoxes('invalid')).toBeNull()
      expect(parseTargetBBoxes('[]')).toBeNull()
    })

    it('parses legacy array format', () => {
      const res = parseTargetBBoxes('[0.1, 0.2, 0.4, 0.6]')
      expect(res).toEqual({
        body: { x1: 0.1, y1: 0.2, x2: 0.4, y2: 0.6 },
        face: undefined,
      })
    })

    it('parses composite format with body and face details', () => {
      const json = JSON.stringify({
        body: { x1: 0.1, y1: 0.2, x2: 0.4, y2: 0.6 },
        face: {
          bbox: { x1: 0.15, y1: 0.22, x2: 0.25, y2: 0.35 },
          confidence: 0.95,
          qualityScore: 0.88,
        },
      })
      const res = parseTargetBBoxes(json)
      expect(res).toEqual({
        body: { x1: 0.1, y1: 0.2, x2: 0.4, y2: 0.6 },
        face: {
          bbox: { x1: 0.15, y1: 0.22, x2: 0.25, y2: 0.35 },
          confidence: 0.95,
          qualityScore: 0.88,
        },
      })
    })

    it('parses composite format with array bboxes and snake_case quality_score', () => {
      const json = JSON.stringify({
        body: [0.1, 0.2, 0.4, 0.6],
        face: {
          bbox: [0.15, 0.22, 0.25, 0.35],
          quality_score: 0.91,
        },
      })
      const res = parseTargetBBoxes(json)
      expect(res).toEqual({
        body: { x1: 0.1, y1: 0.2, x2: 0.4, y2: 0.6 },
        face: {
          bbox: { x1: 0.15, y1: 0.22, x2: 0.25, y2: 0.35 },
          confidence: undefined,
          qualityScore: 0.91,
        },
      })
    })
  })

  describe('getRuleTypeLabel', () => {
    it('maps line to lineCrossing translation', () => {
      const mockT = (k: string) => k
      expect(getRuleTypeLabel('line', mockT)).toBe('types.lineCrossing')
      expect(getRuleTypeLabel('region', mockT)).toBe('types.regionIntrusion')
    })
  })

  describe('calculateFittedImageRect', () => {
    it('handles wider image in square container (pillarbox / horizontal fit)', () => {
      // Container 1000 x 1000, Image 1920 x 1080 (16:9)
      const rect = calculateFittedImageRect(1000, 1000, 1920, 1080)
      expect(rect.width).toBe(1000)
      expect(rect.height).toBeCloseTo(1000 / (1920 / 1080), 1)
      expect(rect.x).toBe(0)
      expect(rect.y).toBeCloseTo((1000 - rect.height) / 2, 1)
    })

    it('handles taller image in square container (letterbox / vertical fit)', () => {
      // Container 1000 x 1000, Image 1080 x 1920 (9:16)
      const rect = calculateFittedImageRect(1000, 1000, 1080, 1920)
      expect(rect.height).toBe(1000)
      expect(rect.width).toBeCloseTo(1000 * (1080 / 1920), 1)
      expect(rect.y).toBe(0)
      expect(rect.x).toBeCloseTo((1000 - rect.width) / 2, 1)
    })
  })

  describe('resolveEffectiveTimeRange', () => {
    const fixedNow = new Date('2026-03-20T14:30:00.000Z').getTime()

    it('resolves "all" preset to undefined boundaries', () => {
      const res = resolveEffectiveTimeRange({ quickPreset: 'all' }, fixedNow)
      expect(res.startTime).toBeUndefined()
      expect(res.endTime).toBeUndefined()
    })

    it('preserves exact timestamps for "custom" preset', () => {
      const res = resolveEffectiveTimeRange(
        { quickPreset: 'custom', startTime: 1000, endTime: 2000 },
        fixedNow,
      )
      expect(res.startTime).toBe(1000)
      expect(res.endTime).toBe(2000)
    })

    it('resolves "today" preset starting at 00:00:00 and without upper bound limitation', () => {
      const res = resolveEffectiveTimeRange({ quickPreset: 'today' }, fixedNow)
      const expectedStart = new Date(fixedNow)
      expectedStart.setHours(0, 0, 0, 0)
      expect(res.startTime).toBe(expectedStart.getTime())
      expect(res.endTime).toBeUndefined()
    })

    it('resolves relative duration presets dynamically based on now', () => {
      const res5m = resolveEffectiveTimeRange({ quickPreset: '5m' }, fixedNow)
      expect(res5m.startTime).toBe(fixedNow - 5 * 60_000)
      expect(res5m.endTime).toBe(fixedNow)

      const res1h = resolveEffectiveTimeRange({ quickPreset: '1h' }, fixedNow)
      expect(res1h.startTime).toBe(fixedNow - 3600_000)
      expect(res1h.endTime).toBe(fixedNow)
    })
  })
})
