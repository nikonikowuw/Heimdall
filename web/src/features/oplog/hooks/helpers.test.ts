import { describe, expect, it } from 'vitest'
import { ApiError } from '@/lib/api'
import type { OperationalLog } from '@/types'
import { getErrorMessage, normalizeOperationalLog } from './helpers'

function buildLog(overrides: Partial<OperationalLog> = {}): OperationalLog {
  return {
    id: 1,
    tsMs: 1747584000000,
    level: 'info',
    event: 'camera_online',
    target: 'media',
    message: '摄像头 cam-01 连接就绪',
    cameraId: 'cam-01',
    extraJson: null,
    ...overrides,
  }
}

describe('getErrorMessage', () => {
  it('takes the message from ApiError and Error', () => {
    expect(getErrorMessage(new ApiError('Unauthorized access', 401))).toBe('Unauthorized access')
    expect(getErrorMessage(new Error('Network disconnected'))).toBe('Network disconnected')
  })

  it('falls back to a non-empty message for arbitrary thrown values', () => {
    expect(getErrorMessage('Server unreachable')).toBe('Server unreachable')
    expect(getErrorMessage({ message: 'Custom error payload' })).toBe('Custom error payload')
    expect(getErrorMessage(null)).toBe('Unknown error')
    expect(getErrorMessage(undefined)).toBe('Unknown error')
    expect(getErrorMessage(12345)).toBe('Unknown error')
    expect(getErrorMessage('')).toBe('Unknown error')
  })
})

describe('normalizeOperationalLog', () => {
  it('keeps the three levels the server contract produces', () => {
    for (const level of ['info', 'warn', 'error'] as const) {
      expect(normalizeOperationalLog(buildLog({ level })).level).toBe(level)
    }
  })

  it('collapses levels outside the contract instead of trusting the wire value', () => {
    const malformed = { ...buildLog(), level: 'fatal' } as unknown as OperationalLog
    expect(normalizeOperationalLog(malformed).level).toBe('info')
    // 服务端写入小写，大写或本地化文案都不属于该契约
    const upperCase = { ...buildLog(), level: 'WARN' } as unknown as OperationalLog
    expect(normalizeOperationalLog(upperCase).level).toBe('info')
  })

  it('leaves the remaining fields untouched', () => {
    const log = buildLog({ extraJson: '{"reason":"timeout"}', cameraId: null })
    expect(normalizeOperationalLog(log)).toEqual(log)
  })
})
