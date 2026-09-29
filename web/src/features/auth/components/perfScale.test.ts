import { describe, expect, it } from 'vitest'
import { nextPerfScale } from './perfScale'

describe('nextPerfScale', () => {
  it('低帧率时下调一档，且不产生浮点尘埃', () => {
    expect(nextPerfScale(1, 18)).toBe(0.85)
    // 0.85 - 0.15 在 IEEE 754 下是 0.7000000000000001，必须收敛回 0.7
    expect(nextPerfScale(0.85, 18)).toBe(0.7)
  })

  it('不跌破下限，也不越过上限', () => {
    expect(nextPerfScale(0.7, 5)).toBe(0.65)
    expect(nextPerfScale(0.65, 10)).toBe(0.65)
    expect(nextPerfScale(1, 60)).toBe(1)
  })

  it('帧率居中时维持档位（滞回区）', () => {
    expect(nextPerfScale(0.85, 30)).toBe(0.85)
    expect(nextPerfScale(1, 45)).toBe(1)
  })

  it('仅在升档折算后仍有余量时才回升', () => {
    // 0.85 → 0.95：像素量约增至 1.25 倍，55 FPS 折算后约 44 FPS，高于判决线 1.5 倍
    expect(nextPerfScale(0.85, 55)).toBe(0.95)
    // 40 FPS 折算后约 32 FPS，没有余量，维持原档，避免反复横跳
    expect(nextPerfScale(0.85, 40)).toBe(0.85)
    // 下限档位同样需要余量：40 FPS 折算后约 30 FPS，不足以升档
    expect(nextPerfScale(0.65, 40)).toBe(0.65)
  })

  it('帧率不可测时维持档位', () => {
    expect(nextPerfScale(0.7, 0)).toBe(0.7)
  })
})
