import { describe, expect, it } from 'vitest'
import { getVideoContentRect } from './videoContentRect'

describe('getVideoContentRect', () => {
  it('centers a contained source and preserves its aspect ratio', () => {
    expect(getVideoContentRect(800, 600, 1920, 1080, 'contain')).toEqual({
      x: 0,
      y: 75,
      width: 800,
      height: 450,
    })
  })

  it('returns the cropped source rect for cover mode', () => {
    const rect = getVideoContentRect(800, 600, 1920, 1080, 'cover')
    expect(rect?.x).toBeCloseTo(-133.3333)
    expect(rect?.y).toBe(0)
    expect(rect?.width).toBeCloseTo(1066.6667)
    expect(rect?.height).toBe(600)
  })

  it('fills the viewport without preserving the source aspect ratio', () => {
    expect(getVideoContentRect(800, 600, 1920, 1080, 'fill')).toEqual({
      x: 0,
      y: 0,
      width: 800,
      height: 600,
    })
  })

  it('returns null until both source and viewport dimensions are known', () => {
    expect(getVideoContentRect(800, 600, 0, 1080, 'contain')).toBeNull()
    expect(getVideoContentRect(0, 600, 1920, 1080, 'cover')).toBeNull()
  })
})
