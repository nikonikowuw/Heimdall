import { describe, expect, it, vi } from 'vitest'
import { telemetryStore } from '@/lib/telemetryStore'
import type { CameraTelemetry } from '@/types'
import { getCameraTelemetrySnapshot, subscribeCameraTelemetry } from './useCameraTelemetry'

describe('useCameraTelemetry helpers', () => {
  it('returns undefined snapshot when no telemetry exists', () => {
    expect(getCameraTelemetrySnapshot('CAM-NON-EXISTENT')).toBeUndefined()
    expect(getCameraTelemetrySnapshot(undefined)).toBeUndefined()
  })

  it('retrieves telemetry snapshot correctly after store updates', () => {
    const mockTelemetry: CameraTelemetry = {
      cameraId: 'CAM-SNAPSHOT-01',
      timestamp: Date.now(),
      activeTracks: 4,
      personCount: 3,
      carCount: 1,
      motionScore: 0.88,
      isMotionGated: false,
    }

    telemetryStore.setTelemetry(mockTelemetry)
    const snapshot = getCameraTelemetrySnapshot('CAM-SNAPSHOT-01')
    expect(snapshot).toEqual(mockTelemetry)
  })

  it('notifies subscribers on store update', () => {
    const listener = vi.fn()
    const unsubscribe = subscribeCameraTelemetry('CAM-NOTIFY-01', listener)

    const mockTelemetry: CameraTelemetry = {
      cameraId: 'CAM-NOTIFY-01',
      timestamp: Date.now(),
      activeTracks: 2,
      personCount: 1,
      carCount: 1,
      motionScore: 0.42,
      isMotionGated: false,
    }

    telemetryStore.setTelemetry(mockTelemetry)
    expect(listener).toHaveBeenCalled()

    unsubscribe()
  })

  it('handles undefined camera gracefully', () => {
    const listener = vi.fn()
    const unsubscribe = subscribeCameraTelemetry(undefined, listener)
    expect(typeof unsubscribe).toBe('function')
    unsubscribe()
  })
})
