import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { CameraTelemetry } from '../types'
import { telemetryStore } from './telemetryStore'

describe('telemetryStore', () => {
  beforeEach(() => {
    telemetryStore.clear()
  })

  it('stores valid telemetry and notifies camera subscribers', () => {
    const listener = vi.fn()
    const unsubscribe = telemetryStore.subscribe('CAM-01', listener)
    const telemetry: CameraTelemetry = {
      cameraId: 'CAM-01',
      timestamp: 1741100000000,
      activeTracks: 2,
      personCount: 1,
      carCount: 1,
      motionScore: 0.65,
      isMotionGated: false,
    }

    telemetryStore.setTelemetry(telemetry)

    expect(telemetryStore.getTelemetry('CAM-01')).toEqual(telemetry)
    expect(listener).toHaveBeenCalledTimes(1)
    unsubscribe()
  })

  it('expires stale telemetry and notifies subscribers', () => {
    vi.useFakeTimers()
    vi.setSystemTime(1000)
    const listener = vi.fn()
    const unsubscribe = telemetryStore.subscribe('CAM-02', listener)
    const telemetry: CameraTelemetry = {
      cameraId: 'CAM-02',
      timestamp: 1741100000000,
      activeTracks: 0,
      personCount: 0,
      carCount: 0,
      motionScore: 0,
      isMotionGated: true,
    }

    telemetryStore.setTelemetry(telemetry)
    vi.advanceTimersByTime(1600)

    expect(telemetryStore.getTelemetry('CAM-02')).toBeUndefined()
    expect(listener).toHaveBeenCalledTimes(2)
    unsubscribe()
    vi.useRealTimers()
  })
})
