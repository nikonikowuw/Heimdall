import { describe, expect, it } from 'vitest'
import type { TrackedBBox } from '../types'
import { TrackRingBuffer } from './trackRingBuffer'

describe('TrackRingBuffer', () => {
  const createMockTrack = (trackId: number, x1 = 0.1): TrackedBBox => ({
    trackId,
    label: 'face',
    confidence: 0.95,
    bbox: [x1, 0.2, x1 + 0.1, 0.4],
  })

  it('should push and lookup exact PTS match', () => {
    const ring = new TrackRingBuffer(10)
    const track1 = [createMockTrack(1, 0.1)]
    const track2 = [createMockTrack(2, 0.2)]

    ring.push(1000, track1)
    ring.push(1050, track2)

    expect(ring.lookup(1000)).toEqual(track1)
    expect(ring.lookup(1050)).toEqual(track2)
  })

  it('should lookup closest PTS within tolerance window', () => {
    const ring = new TrackRingBuffer(10)
    const track1 = [createMockTrack(1, 0.1)] // pts = 1000
    const track2 = [createMockTrack(2, 0.2)] // pts = 1100

    ring.push(1000, track1)
    ring.push(1100, track2)

    // 1040 距离 1000 差 40ms，距离 1100 差 60ms -> 匹配 track1
    expect(ring.lookup(1040, 150)).toEqual(track1)

    // 1070 距离 1000 差 70ms，距离 1100 差 30ms -> 匹配 track2
    expect(ring.lookup(1070, 150)).toEqual(track2)
  })

  it('should return empty tracks when beyond tolerance window', () => {
    const ring = new TrackRingBuffer(10)
    const track1 = [createMockTrack(1, 0.1)] // pts = 1000

    ring.push(1000, track1)

    // 视频时间差 200ms > 容差 150ms -> 判定为失步，返回空，避免绘制幽灵框
    expect(ring.lookup(1200, 150)).toEqual([])
    expect(ring.lookup(800, 150)).toEqual([])
  })

  it('should handle out-of-order PTS insertions gracefully', () => {
    const ring = new TrackRingBuffer(10)
    const track1 = [createMockTrack(1)]
    const track2 = [createMockTrack(2)]
    const track3 = [createMockTrack(3)]

    // 乱序到达：1100, 1000, 1050
    ring.push(1100, track3)
    ring.push(1000, track1)
    ring.push(1050, track2)

    expect(ring.size).toBe(3)
    expect(ring.lookup(1000)).toEqual(track1)
    expect(ring.lookup(1050)).toEqual(track2)
    expect(ring.lookup(1100)).toEqual(track3)
  })

  it('should enforce maximum bounded capacity and evict oldest frames', () => {
    const ring = new TrackRingBuffer(3)

    ring.push(100, [createMockTrack(1)])
    ring.push(200, [createMockTrack(2)])
    ring.push(300, [createMockTrack(3)])
    ring.push(400, [createMockTrack(4)]) // 应该将 100 挤出

    expect(ring.size).toBe(3)
    // 100 已被挤出，lookup 应该找不到 (距离最近的 200 差 100ms，在 50ms 容差下返回空)
    expect(ring.lookup(100, 50)).toEqual([])
    expect(ring.lookup(400, 50)).toEqual([createMockTrack(4)])
  })

  it('should support getLatest fallback when no video PTS is available', () => {
    const ring = new TrackRingBuffer(10)
    expect(ring.getLatest()).toEqual([])

    const track1 = [createMockTrack(1)]
    const track2 = [createMockTrack(2)]
    ring.push(1000, track1)
    ring.push(1050, track2)

    expect(ring.getLatest()).toEqual(track2)
  })

  it('should update in-place for identical PTS to maintain strict monotonicity', () => {
    const ring = new TrackRingBuffer(10)
    const track1 = [createMockTrack(1, 0.1)]
    const track2 = [createMockTrack(2, 0.2)]

    // 1. 连续到达相同时间戳 (如多算法聚合或重传)
    ring.push(1000, track1)
    ring.push(1000, track2)
    expect(ring.size).toBe(1)
    expect(ring.lookup(1000)).toEqual(track2)

    // 2. 乱序到达相同时间戳
    const track3 = [createMockTrack(3, 0.3)]
    const track4 = [createMockTrack(4, 0.4)]
    ring.push(1200, track3)
    ring.push(1000, track4) // 乱序回补更新 1000ms
    expect(ring.size).toBe(2)
    expect(ring.lookup(1000)).toEqual(track4)
  })

  it('should ignore invalid PTS values in push and lookup', () => {
    const ring = new TrackRingBuffer(10)
    ring.push(NaN, [createMockTrack(1)])
    ring.push(-100, [createMockTrack(2)])
    ring.push(0, [createMockTrack(3)])

    expect(ring.size).toBe(0)
    expect(ring.lookup(100)).toEqual([])

    // lookup 输入校验防护
    ring.push(1000, [createMockTrack(1)])
    expect(ring.lookup(0)).toEqual([])
    expect(ring.lookup(-500)).toEqual([])
    expect(ring.lookup(NaN)).toEqual([])
  })
})
