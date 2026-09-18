import { describe, expect, it } from 'vitest'
import { calcZoomIn, calcZoomOut, clampZoom } from './useImageZoomPan'

describe('useImageZoomPan math calculations', () => {
  it('clamps zoom value correctly', () => {
    expect(clampZoom(0.5, 1, 5)).toBe(1)
    expect(clampZoom(3.14159, 1, 5)).toBe(3.14)
    expect(clampZoom(6.5, 1, 5)).toBe(5)
  })

  it('calculates zoom in progression', () => {
    expect(calcZoomIn(1, 0.3, 5)).toBe(1.3)
    expect(calcZoomIn(4.8, 0.3, 5)).toBe(5)
    expect(calcZoomIn(5, 0.3, 5)).toBe(5)
  })

  it('calculates zoom out progression', () => {
    expect(calcZoomOut(2.2, 0.3, 1)).toBe(1.9)
    expect(calcZoomOut(1.2, 0.3, 1)).toBe(1)
    expect(calcZoomOut(1, 0.3, 1)).toBe(1)
  })
})
