import { describe, expect, it } from 'vitest'
import {
  getActiveAlgorithmIds,
  getCameraAiRuntimeStatus,
  getProbeBadge,
  getProbeButtonText,
  normalizeProbeStatus,
} from './cameraStatus'

describe('cameraStatus helper', () => {
  it('normalizeProbeStatus maps statuses correctly', () => {
    expect(normalizeProbeStatus('healthy')).toBe('healthy')
    expect(normalizeProbeStatus('success')).toBe('healthy')
    expect(normalizeProbeStatus('online')).toBe('healthy')
    expect(normalizeProbeStatus('degraded')).toBe('degraded')
    expect(normalizeProbeStatus('reconnecting')).toBe('degraded')
    expect(normalizeProbeStatus('failed')).toBe('offline')
    expect(normalizeProbeStatus('offline')).toBe('offline')
    expect(normalizeProbeStatus('error')).toBe('offline')
    expect(normalizeProbeStatus('pending')).toBe('unprobed')
    expect(normalizeProbeStatus(null)).toBe('unprobed')
    expect(normalizeProbeStatus(undefined)).toBe('unprobed')
  })

  it('getProbeBadge returns badge styling and fallback text', () => {
    const healthyBadge = getProbeBadge('healthy')
    expect(healthyBadge.status).toBe('healthy')
    expect(healthyBadge.text).toBe('在线')
    expect(healthyBadge.dotClass).toContain('bg-emerald-500')

    const offlineBadge = getProbeBadge('offline')
    expect(offlineBadge.status).toBe('offline')
    expect(offlineBadge.text).toBe('离线')
    expect(offlineBadge.dotClass).toContain('bg-rose-500')

    const unprobedBadge = getProbeBadge('unknown')
    expect(unprobedBadge.status).toBe('unprobed')
    expect(unprobedBadge.text).toBe('待探测')
  })

  it('getProbeBadge supports custom translation resolver', () => {
    const customT = (key: string) => (key === 'card.onlineStatus' ? 'Custom Online' : key)
    const badge = getProbeBadge('healthy', customT)
    expect(badge.text).toBe('Custom Online')
  })

  it('derives AI runtime status and de-duplicates enabled algorithms', () => {
    const task = {
      desiredEnabled: true,
      actualStatus: 2,
      algorithmId: 'fallback',
      analysisFps: 10,
      algorithmInstanceCount: 3,
      algorithmInstances: [
        {
          instanceId: 'a',
          algorithmId: 'person',
          analysisFps: 10,
          enabled: true,
          actualStatus: 2,
          applyState: 'applied' as const,
          statusMessage: '',
        },
        {
          instanceId: 'b',
          algorithmId: ' person ',
          analysisFps: 10,
          enabled: true,
          actualStatus: 2,
          applyState: 'applied' as const,
          statusMessage: '',
        },
        {
          instanceId: 'c',
          algorithmId: 'vehicle',
          analysisFps: 10,
          enabled: false,
          actualStatus: 0,
          applyState: 'applied' as const,
          statusMessage: '',
        },
      ],
      updatedAt: 1710000000000,
      statusMessage: '',
    }

    expect(getActiveAlgorithmIds(task)).toEqual(['person'])
    expect(getCameraAiRuntimeStatus(task)).toMatchObject({
      status: 'active',
      isActive: true,
      activeAlgorithmIds: ['person'],
    })
  })

  it('keeps configured but stopped, starting, and failed pipelines distinguishable', () => {
    const baseTask = {
      desiredEnabled: true,
      algorithmId: 'person',
      algorithmInstanceCount: 1,
      algorithmInstances: [],
      analysisFps: 10,
      updatedAt: 1710000000000,
      statusMessage: '',
    }

    expect(getCameraAiRuntimeStatus({ ...baseTask, actualStatus: 0 }).status).toBe('inactive')
    expect(getCameraAiRuntimeStatus({ ...baseTask, actualStatus: 1 }).status).toBe('starting')
    expect(getCameraAiRuntimeStatus({ ...baseTask, actualStatus: 3 }).status).toBe('degraded')
    expect(getCameraAiRuntimeStatus({ ...baseTask, actualStatus: 5 }).status).toBe('error')
    expect(
      getCameraAiRuntimeStatus({ ...baseTask, desiredEnabled: false, actualStatus: 2 }).status,
    ).toBe('inactive')
  })

  it('resolves probe button text across probing, success, failure and idle states', () => {
    const fakeT = (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue || key

    expect(getProbeButtonText(true, undefined, fakeT)).toBe('探活中...')
    expect(getProbeButtonText(false, 'success', fakeT)).toBe('探活成功')
    expect(getProbeButtonText(false, 'failed', fakeT)).toBe('探活失败')
    expect(getProbeButtonText(false, undefined, fakeT)).toBe('探活')
    expect(getProbeButtonText(false, undefined, fakeT, 'tile.probeAction')).toBe('探活')
  })
})
