import { describe, expect, it } from 'vitest'
import { getProbeBadge, normalizeProbeStatus } from './cameraStatus'

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
    expect(healthyBadge.text).toBe('正常在线')
    expect(healthyBadge.dotClass).toContain('bg-emerald-500')

    const offlineBadge = getProbeBadge('offline')
    expect(offlineBadge.status).toBe('offline')
    expect(offlineBadge.text).toBe('设备离线')
    expect(offlineBadge.dotClass).toContain('bg-rose-500')

    const unprobedBadge = getProbeBadge('unknown')
    expect(unprobedBadge.status).toBe('unprobed')
    expect(unprobedBadge.text).toBe('待探活')
  })

  it('getProbeBadge supports custom translation resolver', () => {
    const customT = (key: string) => (key === 'card.onlineStatus' ? 'Custom Online' : key)
    const badge = getProbeBadge('healthy', customT)
    expect(badge.text).toBe('Custom Online')
  })
})
