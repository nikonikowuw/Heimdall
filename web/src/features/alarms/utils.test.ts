import { describe, expect, it } from 'vitest'
import {
  calculateFittedImageRect,
  formatTimestamp,
  getRuleTypeLabel,
  parseBBoxCoords,
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
})
