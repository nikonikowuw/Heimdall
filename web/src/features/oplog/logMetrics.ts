import type { OperationLog, OperationalLog } from '@/types'
import { classifyHttpStatus, classifyLatency, type LatencyTier } from './logTone'

/**
 * 审计日志的指标口径是「当前页」。服务端没有聚合端点，把整表总量当作卡片数值
 * 会把一页数据伪装成全量结果，因此这里显式按页统计，文案也标明本页范围。
 */
export interface OperationLogMetrics {
  pageSize: number
  successCount: number
  /** 4xx 与 5xx 合计 */
  errorCount: number
  /** 百分比字面量，例如 `98.0%` */
  successRate: string
  avgDurationMs: number
  latencyTier: LatencyTier
}

export function summarizeOperationLogs(logs: OperationLog[]): OperationLogMetrics {
  let successCount = 0
  let errorCount = 0
  let totalDurationMs = 0

  for (const log of logs) {
    const statusClass = classifyHttpStatus(log.statusCode)
    if (statusClass === 'success') successCount += 1
    if (statusClass === 'clientError' || statusClass === 'serverError') errorCount += 1
    totalDurationMs += log.durationMs
  }

  const pageSize = logs.length
  const avgDurationMs = pageSize === 0 ? 0 : Math.round(totalDurationMs / pageSize)

  return {
    pageSize,
    successCount,
    errorCount,
    successRate: pageSize === 0 ? '100%' : `${((successCount / pageSize) * 100).toFixed(1)}%`,
    avgDurationMs,
    latencyTier: classifyLatency(avgDurationMs),
  }
}

export interface OperationalLogMetrics {
  pageSize: number
  errorCount: number
  warnCount: number
  infoCount: number
}

export function summarizeOperationalLogs(logs: OperationalLog[]): OperationalLogMetrics {
  let errorCount = 0
  let warnCount = 0
  let infoCount = 0

  for (const log of logs) {
    switch (log.level) {
      case 'error':
        errorCount += 1
        break
      case 'warn':
        warnCount += 1
        break
      case 'info':
        infoCount += 1
        break
    }
  }

  return {
    pageSize: logs.length,
    errorCount,
    warnCount,
    infoCount,
  }
}
