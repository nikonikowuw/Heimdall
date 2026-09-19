import { ApiError } from '@/lib/api'
import { isMemberOf, LEVEL_VALUES } from '@/features/oplog/logFilters'
import type { OperationalLog } from '@/types'

/**
 * 统一从各种异常对象中提取安全可读的错误文案
 */
export function getErrorMessage(error: unknown): string {
  if (error instanceof ApiError || error instanceof Error) {
    return error.message || 'Unknown error'
  }
  if (typeof error === 'string' && error.trim().length > 0) {
    return error
  }
  if (
    error &&
    typeof error === 'object' &&
    'message' in error &&
    typeof error.message === 'string' &&
    error.message.trim().length > 0
  ) {
    return error.message
  }
  return 'Unknown error'
}

/**
 * 网络边界归一化：服务端契约只产生 info/warn/error（见 OpEvent::level），
 * 越界输入收敛为 info，避免任意字符串进入类型体系与色调映射。
 */
export function normalizeOperationalLog(raw: OperationalLog): OperationalLog {
  return { ...raw, level: isMemberOf(LEVEL_VALUES, raw.level) ? raw.level : 'info' }
}
