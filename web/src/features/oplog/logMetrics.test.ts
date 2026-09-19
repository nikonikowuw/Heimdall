import { describe, expect, it } from 'vitest'
import type { OperationLog, OperationalLog } from '@/types'
import { summarizeOperationLogs, summarizeOperationalLogs } from './logMetrics'

function buildOperationLog(statusCode: number, durationMs: number): OperationLog {
  return {
    id: statusCode * 1000 + durationMs,
    username: 'admin',
    module: 'camera',
    action: 'create',
    method: 'POST',
    path: '/api/v1/cameras',
    query: '',
    body: '',
    statusCode,
    durationMs,
    ip: '127.0.0.1',
    userAgent: 'test',
    createdAt: 1747584000000,
  }
}

function buildOperationalLog(level: OperationalLog['level']): OperationalLog {
  return {
    id: 1,
    tsMs: 1747584000000,
    level,
    event: 'camera_offline',
    target: 'media',
    message: '摄像头离线',
    cameraId: null,
    extraJson: null,
  }
}

describe('summarizeOperationLogs', () => {
  it('reports an empty page without dividing by zero', () => {
    expect(summarizeOperationLogs([])).toEqual({
      pageSize: 0,
      successCount: 0,
      errorCount: 0,
      successRate: '100%',
      avgDurationMs: 0,
      latencyTier: 'fast',
    })
  })

  it('counts 4xx and 5xx together as errors and keeps 3xx out of both buckets', () => {
    const metrics = summarizeOperationLogs([
      buildOperationLog(200, 10),
      buildOperationLog(302, 10),
      buildOperationLog(404, 10),
      buildOperationLog(500, 10),
    ])

    expect(metrics.pageSize).toBe(4)
    expect(metrics.successCount).toBe(1)
    expect(metrics.errorCount).toBe(2)
    expect(metrics.successRate).toBe('25.0%')
  })

  it('averages the durations of the current page and rounds to whole milliseconds', () => {
    const metrics = summarizeOperationLogs([
      buildOperationLog(200, 10),
      buildOperationLog(200, 11),
      buildOperationLog(200, 12),
    ])

    expect(metrics.avgDurationMs).toBe(11)
    expect(metrics.latencyTier).toBe('fast')
  })

  it('derives the latency tier from the same thresholds as the table', () => {
    const metrics = summarizeOperationLogs([
      buildOperationLog(200, 600),
      buildOperationLog(200, 600),
    ])

    expect(metrics.avgDurationMs).toBe(600)
    expect(metrics.latencyTier).toBe('slow')
  })
})

describe('summarizeOperationalLogs', () => {
  it('splits the page into the three level buckets', () => {
    const metrics = summarizeOperationalLogs([
      buildOperationalLog('info'),
      buildOperationalLog('info'),
      buildOperationalLog('warn'),
      buildOperationalLog('error'),
    ])

    expect(metrics).toEqual({
      pageSize: 4,
      errorCount: 1,
      warnCount: 1,
      infoCount: 2,
    })
  })

  it('reports zero buckets for an empty page', () => {
    expect(summarizeOperationalLogs([])).toEqual({
      pageSize: 0,
      errorCount: 0,
      warnCount: 0,
      infoCount: 0,
    })
  })
})
