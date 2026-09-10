import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useAuthStore } from '../stores/auth'
import { type CameraTracksPayload, WS_TOPICS } from '../types'
import { trackStore } from './trackStore'
import { wsClient } from './wsClient'

describe('wsClient', () => {
  beforeEach(() => {
    trackStore.clear()
    useAuthStore.setState({
      token: null,
      username: null,
      isAuthenticated: false,
    })
  })

  it('should subscribe and notify topic listeners on message', () => {
    const listener = vi.fn()
    const unsubscribe = wsClient.subscribe('custom.topic', listener)

    const mockPayload = { foo: 'bar' }
    wsClient.dispatch('custom.topic', mockPayload, 1741100000000)

    expect(listener).toHaveBeenCalledWith(mockPayload, 1741100000000)
    unsubscribe()
  })

  it('should automatically feed camera.tracks into trackStore', () => {
    wsClient.init()

    const mockTracksPayload: CameraTracksPayload = {
      cameraId: 'CAM-WS-01',
      timestamp: 1741100000000,
      tracks: [
        {
          trackId: 99,
          label: 'person',
          confidence: 0.99,
          bbox: [0.1, 0.2, 0.3, 0.4],
        },
      ],
    }

    wsClient.dispatch(WS_TOPICS.CAMERA_TRACKS, mockTracksPayload, 1741100000000)

    expect(trackStore.getTracks('CAM-WS-01')).toEqual(mockTracksPayload.tracks)
  })

  it('should isolate subscriber callback errors and continue dispatching', () => {
    const faultyListener = vi.fn(() => {
      throw new Error('Subscriber explode')
    })
    const healthyListener = vi.fn()

    const unFaulty = wsClient.subscribe('resilient.topic', faultyListener)
    const unHealthy = wsClient.subscribe('resilient.topic', healthyListener)

    wsClient.dispatch('resilient.topic', { ok: true }, 1741100000000)

    expect(faultyListener).toHaveBeenCalled()
    expect(healthyListener).toHaveBeenCalledWith({ ok: true }, 1741100000000)

    unFaulty()
    unHealthy()
  })
})
