import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { TrackedBBox } from '../types'
import { trackStore } from './trackStore'

describe('trackStore', () => {
  beforeEach(() => {
    trackStore.clear()
  })

  it('should set and get tracks for camera', () => {
    const mockTracks: TrackedBBox[] = [
      {
        trackId: 1,
        label: 'person',
        confidence: 0.95,
        bbox: [0.1, 0.2, 0.3, 0.4],
      },
    ]

    trackStore.setTracks('CAM-01', mockTracks)
    expect(trackStore.getTracks('CAM-01')).toEqual(mockTracks)
    expect(trackStore.getTracks('CAM-02')).toEqual([])
  })

  it('should notify subscribers on track updates', () => {
    const listener = vi.fn()
    const unsubscribe = trackStore.subscribe('CAM-01', listener)

    const mockTracks: TrackedBBox[] = [
      {
        trackId: 2,
        label: 'car',
        confidence: 0.88,
        bbox: [0.2, 0.3, 0.5, 0.6],
      },
    ]

    trackStore.setTracks('CAM-01', mockTracks)
    expect(listener).toHaveBeenCalledWith(mockTracks)

    unsubscribe()
    trackStore.setTracks('CAM-01', [])
    expect(listener).toHaveBeenCalledTimes(1)
  })

  it('should immediately emit existing snapshot on subscribe', () => {
    const existing: TrackedBBox[] = [
      {
        trackId: 3,
        label: 'person',
        confidence: 0.9,
        bbox: [0.1, 0.1, 0.2, 0.2],
      },
    ]
    trackStore.setTracks('CAM-02', existing)

    const listener = vi.fn()
    const unsubscribe = trackStore.subscribe('CAM-02', listener)
    expect(listener).toHaveBeenCalledWith(existing)

    unsubscribe()
  })

  it('should not emit expired snapshot on subscribe after TTL', () => {
    vi.useFakeTimers()
    const existing: TrackedBBox[] = [
      {
        trackId: 4,
        label: 'person',
        confidence: 0.9,
        bbox: [0.1, 0.1, 0.2, 0.2],
      },
    ]
    trackStore.setTracks('CAM-03', existing)

    // 快进时间超过 1500ms TTL
    vi.advanceTimersByTime(2000)

    const listener = vi.fn()
    const unsubscribe = trackStore.subscribe('CAM-03', listener)
    expect(listener).not.toHaveBeenCalled()
    expect(trackStore.getTracks('CAM-03')).toEqual([])

    unsubscribe()
    vi.useRealTimers()
  })

  it('should support PTS-aligned track lookup via ring buffer', () => {
    const trackT1: TrackedBBox[] = [
      {
        trackId: 10,
        label: 'face',
        confidence: 0.96,
        bbox: [0.1, 0.1, 0.2, 0.2],
      },
    ]
    const trackT2: TrackedBBox[] = [
      {
        trackId: 10,
        label: 'face',
        confidence: 0.98,
        bbox: [0.3, 0.3, 0.4, 0.4],
      },
    ]

    // 写入带 PTS 的航迹流 (pts = 1000ms 与 1040ms)
    trackStore.setTracks('CAM-PTS', trackT1, 1000)
    trackStore.setTracks('CAM-PTS', trackT2, 1040)

    // 1. 根据当前视频帧的 PTS (1002ms) 查找：匹配 trackT1
    expect(trackStore.getTracks('CAM-PTS', 1002)).toEqual(trackT1)

    // 2. 根据当前视频帧的 PTS (1038ms) 查找：匹配 trackT2
    expect(trackStore.getTracks('CAM-PTS', 1038)).toEqual(trackT2)

    // 3. 不带 PTS 查询时回退到最新快照
    expect(trackStore.getTracks('CAM-PTS')).toEqual(trackT2)

    // 4. 清除后查询为空
    trackStore.clear('CAM-PTS')
    expect(trackStore.getTracks('CAM-PTS', 1040)).toEqual([])
  })
})
