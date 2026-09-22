import { describe, expect, it } from 'vitest'
import {
  calculateFittedImageRect,
  captureEvidencePath,
  deriveEvidenceOriginBadges,
  foldForSearch,
  formatFaceBBoxLabel,
  formatTimestamp,
  getRuleTypeLabel,
  matchesSearchTerm,
  parseBBoxCoords,
  parseTargetBBoxes,
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

    it('parses face-only format without body (backend pseudo-body stripped)', () => {
      const json = JSON.stringify({
        face: {
          bbox: { x1: 0.3, y1: 0.2, x2: 0.5, y2: 0.45 },
          confidence: 0.98,
          qualityScore: 0.85,
          pseudoBody: true,
        },
      })
      const res = parseTargetBBoxes(json)
      expect(res).toEqual({
        body: undefined,
        face: {
          bbox: { x1: 0.3, y1: 0.2, x2: 0.5, y2: 0.45 },
          confidence: 0.98,
          qualityScore: 0.85,
        },
      })
      // 验证向后兼容 parseBBoxCoords 能够获取到人脸坐标作为兜底主体坐标
      expect(parseBBoxCoords(json)).toEqual({ x1: 0.3, y1: 0.2, x2: 0.5, y2: 0.45 })
    })
  })

  describe('formatFaceBBoxLabel', () => {
    it('renders detection confidence and quality score with distinct prefixes', () => {
      expect(
        formatFaceBBoxLabel({
          bbox: { x1: 0.1, y1: 0.2, x2: 0.3, y2: 0.4 },
          confidence: 0.93,
          qualityScore: 0.42,
        }),
      ).toBe('Face 93% · Q 42%')
    })

    it('omits missing fields instead of reusing one value for the other', () => {
      expect(formatFaceBBoxLabel({ bbox: { x1: 0, y1: 0, x2: 1, y2: 1 } })).toBe('Face')
      expect(
        formatFaceBBoxLabel({
          bbox: { x1: 0, y1: 0, x2: 1, y2: 1 },
          qualityScore: 0.5,
        }),
      ).toBe('Face · Q 50%')
      expect(formatFaceBBoxLabel(undefined)).toBe('Face')
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

  describe('deriveEvidenceOriginBadges', () => {
    it('labels peak candidate frames and sub stream frames in order', () => {
      expect(deriveEvidenceOriginBadges('peak_candidate', 'sub')).toEqual([
        'peakFrame',
        'subStream',
      ])
      expect(deriveEvidenceOriginBadges('peak_candidate', 'main')).toEqual(['peakFrame'])
      expect(deriveEvidenceOriginBadges('targeted', 'sub')).toEqual(['subStream'])
    })

    it('renders nothing for main stream targeted evidence', () => {
      expect(deriveEvidenceOriginBadges('targeted', 'main')).toEqual([])
    })

    it('renders nothing for legacy rows without source markers', () => {
      expect(deriveEvidenceOriginBadges(null, null)).toEqual([])
      expect(deriveEvidenceOriginBadges(undefined, undefined)).toEqual([])
      expect(deriveEvidenceOriginBadges('', '')).toEqual([])
    })

    it('is tolerant to unknown future values from a newer backend', () => {
      expect(deriveEvidenceOriginBadges('main_replay', 'main')).toEqual([])
    })
  })
})

describe('captureEvidencePath 抓拍证据图回退链', () => {
  it('prefers the body crop so a back-facing person is still identifiable by clothing', () => {
    expect(
      captureEvidencePath({
        bodyCropImageRelPath: 'cam/body.jpg',
        cropImageRelPath: 'cam/face.jpg',
        imageRelPath: 'cam/full.jpg',
      }),
    ).toBe('cam/body.jpg')
  })

  it('falls back to the face crop when no body crop was produced', () => {
    expect(
      captureEvidencePath({
        bodyCropImageRelPath: '',
        cropImageRelPath: 'cam/face.jpg',
        imageRelPath: 'cam/full.jpg',
      }),
    ).toBe('cam/face.jpg')
  })

  it('falls back to the panorama for legacy rows predating the body crop column', () => {
    expect(captureEvidencePath({ imageRelPath: 'cam/full.jpg' })).toBe('cam/full.jpg')
  })

  it('returns an empty path when no evidence image is recorded', () => {
    expect(captureEvidencePath({})).toBe('')
  })

  describe('foldForSearch', () => {
    it('folds ASCII letters only, matching SQLite LIKE', () => {
      expect(foldForSearch('Front Gate')).toBe('front gate')
      // SQLite 的 LIKE 不折叠非 ASCII：Ä 与 ä 视为不同字符
      expect(foldForSearch('Ä')).toBe('Ä')
      expect(foldForSearch('ä')).toBe('ä')
      expect(foldForSearch('Ⅰ')).toBe('Ⅰ')
    })

    it('leaves digits, punctuation and CJK untouched', () => {
      expect(foldForSearch('cam_01 100%')).toBe('cam_01 100%')
      expect(foldForSearch('前门通道')).toBe('前门通道')
    })
  })

  describe('matchesSearchTerm', () => {
    it('matches any candidate field case-insensitively for ASCII', () => {
      expect(matchesSearchTerm('gate', ['Front Gate', 'cam_01'])).toBe(true)
      expect(matchesSearchTerm('GATE', ['Front Gate'])).toBe(true)
      expect(matchesSearchTerm('gate', ['Front Gate', 'cam_01'])).toBe(true)
    })

    it('returns true for a blank keyword so unfiltered streams stay visible', () => {
      expect(matchesSearchTerm('', ['anything'])).toBe(true)
      expect(matchesSearchTerm('   ', [])).toBe(true)
    })

    it('returns false when nothing matches', () => {
      expect(matchesSearchTerm('nope', ['Front Gate', 'cam_01'])).toBe(false)
    })

    it('skips null and undefined fields without throwing', () => {
      expect(matchesSearchTerm('gate', [null, undefined, 'Front Gate'])).toBe(true)
      expect(matchesSearchTerm('gate', [null, undefined])).toBe(false)
    })

    it('treats non-ASCII case as distinct, mirroring the server-side LIKE', () => {
      // 若这里用 toLowerCase() 就会误命中，与服务端结果不一致
      expect(matchesSearchTerm('ä', ['Ä'])).toBe(false)
      expect(matchesSearchTerm('Ä', ['Ä'])).toBe(true)
    })
  })
})
