import { useCallback, useEffect, useRef, useState } from 'react'
import type { ModuleFilter, StatusFilter } from '@/features/oplog/logFilters'
import { DEFAULT_LOG_PAGE_SIZE } from '@/features/oplog/logPaging'
import { oplogApi } from '@/lib/api'
import type { OperationLog } from '@/types'
import { getErrorMessage } from './helpers'

export interface UseOplogsFilters {
  /** 业务模块，`all` 表示不过滤 */
  module?: ModuleFilter
  /** 状态码分类，`all` 表示不过滤 */
  status?: StatusFilter
  /** 关键字，命中操作人、路径、模块、动作与客户端 IP */
  keyword?: string
  fromMs?: number
  toMs?: number
}

export interface UseOplogsResult {
  /** 当前页记录；筛选与分页都在服务端完成，因此每页替换而非追加 */
  logs: OperationLog[]
  isLoading: boolean
  hasMore: boolean
  error: string | null
  refresh: () => void
}

/**
 * 操作审计日志分页查询。
 *
 * 全部筛选条件下推服务端，前端不再对已取回的一页做二次过滤——否则会出现
 * 「当前页无匹配但后续页有匹配」的空表误判，且分页计数与真实命中数不一致。
 * 列表按页替换，单页容量受服务端 MAX_LIMIT(100) 与档位限制，天然低于 1000 条上限。
 */
export function useOplogs(
  filters: UseOplogsFilters,
  page: number = 1,
  pageSize: number = DEFAULT_LOG_PAGE_SIZE,
): UseOplogsResult {
  const { module, status, keyword, fromMs, toMs } = filters

  const [logs, setLogs] = useState<OperationLog[]>([])
  const [isLoading, setIsLoading] = useState(true)
  const [hasMore, setHasMore] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [refreshVersion, setRefreshVersion] = useState(0)
  const generationRef = useRef(0)

  const moduleFilter = module === undefined || module === 'all' ? undefined : module
  const statusFilter = status === undefined || status === 'all' ? undefined : status
  const trimmedKeyword = keyword?.trim()
  const keywordFilter = trimmedKeyword ? trimmedKeyword : undefined

  useEffect(() => {
    const generation = generationRef.current + 1
    generationRef.current = generation
    const controller = new AbortController()

    setIsLoading(true)
    setError(null)

    void oplogApi
      .list(
        {
          module: moduleFilter,
          status: statusFilter,
          q: keywordFilter,
          fromMs,
          toMs,
          limit: pageSize,
          offset: Math.max(0, (page - 1) * pageSize),
        },
        controller.signal,
      )
      .then((nextLogs) => {
        if (generationRef.current !== generation) return
        setLogs(nextLogs)
        setHasMore(nextLogs.length === pageSize)
      })
      .catch((requestError: unknown) => {
        if (controller.signal.aborted || generationRef.current !== generation) return
        setError(getErrorMessage(requestError))
      })
      .finally(() => {
        if (generationRef.current === generation) {
          setIsLoading(false)
        }
      })

    return () => {
      controller.abort()
      if (generationRef.current === generation) {
        generationRef.current += 1
      }
    }
  }, [moduleFilter, statusFilter, keywordFilter, fromMs, toMs, page, pageSize, refreshVersion])

  const refresh = useCallback(() => {
    setRefreshVersion((version) => version + 1)
  }, [])

  return {
    logs,
    isLoading,
    hasMore,
    error,
    refresh,
  }
}
