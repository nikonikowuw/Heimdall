import { describe, expect, it } from 'vitest'
import { ApiError } from '../../../lib/api'
import type { OperationLog } from '../../../types'
import { getErrorMessage, mergeLogs } from './useOplogs'

describe('useOplogs helpers', () => {
  it('getErrorMessage extracts error message from ApiError or standard Error', () => {
    expect(getErrorMessage(new ApiError('Unauthorized access', 401))).toBe('Unauthorized access')
    expect(getErrorMessage(new Error('Network disconnected'))).toBe('Network disconnected')
  })

  it('getErrorMessage provides non-empty fallback for arbitrary non-Error objects', () => {
    expect(getErrorMessage('Server unreachable')).toBe('Server unreachable')
    expect(getErrorMessage({ message: 'Custom error payload' })).toBe('Custom error payload')
    expect(getErrorMessage(null)).toBe('Unknown error')
    expect(getErrorMessage(undefined)).toBe('Unknown error')
    expect(getErrorMessage(12345)).toBe('Unknown error')
  })

  it('mergeLogs deduplicates records by id on offset pagination', () => {
    const existing: OperationLog[] = [
      {
        id: 10,
        username: 'admin',
        module: 'camera',
        action: 'create',
        method: 'POST',
        path: '/api/v1/cameras',
        query: '',
        body: '',
        statusCode: 201,
        durationMs: 4,
        ip: '127.0.0.1',
        userAgent: 'test',
        createdAt: 1747584000000,
      },
      {
        id: 9,
        username: 'admin',
        module: 'camera',
        action: 'update',
        method: 'PUT',
        path: '/api/v1/cameras/1',
        query: '',
        body: '',
        statusCode: 200,
        durationMs: 3,
        ip: '127.0.0.1',
        userAgent: 'test',
        createdAt: 1747583900000,
      },
    ]

    const incoming: OperationLog[] = [
      // duplicate id 9
      {
        id: 9,
        username: 'admin',
        module: 'camera',
        action: 'update',
        method: 'PUT',
        path: '/api/v1/cameras/1',
        query: '',
        body: '',
        statusCode: 200,
        durationMs: 3,
        ip: '127.0.0.1',
        userAgent: 'test',
        createdAt: 1747583900000,
      },
      // new id 8
      {
        id: 8,
        username: 'admin',
        module: 'task',
        action: 'delete',
        method: 'DELETE',
        path: '/api/v1/tasks/1',
        query: '',
        body: '',
        statusCode: 200,
        durationMs: 2,
        ip: '127.0.0.1',
        userAgent: 'test',
        createdAt: 1747583800000,
      },
    ]

    const merged = mergeLogs(existing, incoming)
    expect(merged.map((log) => log.id)).toEqual([10, 9, 8])
  })
})
