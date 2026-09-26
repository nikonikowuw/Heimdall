import { useCallback, useEffect, useRef, useState } from 'react'
import type { ModuleFilter, StatusFilter } from '@/features/oplog/logFilters'
import { DEFAULT_LOG_PAGE_SIZE } from '@/features/oplog/logPaging'
import { oplogApi } from '@/lib/api'
import { buildQuerySignature } from '@/lib/utils'
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
  // 已落定的查询签名；与当前查询签名不一致即为加载中。
  //
  // 不再用 `setIsLoading(true)` 开头：effect 内同步 setState 会在每次筛选/翻页时
  // 多触发一轮渲染（先提交 loading，再提交结果），而「是否在加载」完全可由
  // 「请求签名是否已落定」派生。错误同样按签名归属，筛选一变即自动失效，
  // 无需在 effect 开头手动清零。
  const [settledQuery, setSettledQuery] = useState<string | null>(null)
  const [failure, setFailure] = useState<{ query: string; message: string } | null>(null)
  const [hasMore, setHasMore] = useState(false)
  const [refreshVersion, setRefreshVersion] = useState(0)
  const generationRef = useRef(0)

  const moduleFilter = module === undefined || module === 'all' ? undefined : module
  const statusFilter = status === undefined || status === 'all' ? undefined : status
  const trimmedKeyword = keyword?.trim()
  const keywordFilter = trimmedKeyword ? trimmedKeyword : undefined

  // 请求签名：即下方 effect 的依赖集合，用于派生 isLoading 与错误归属
  const query = buildQuerySignature(
    moduleFilter,
    statusFilter,
    keywordFilter,
    fromMs,
    toMs,
    page,
    pageSize,
    refreshVersion,
  )
  const isLoading = settledQuery !== query
  const error = failure && failure.query === query ? failure.message : null

  useEffect(() => {
    const generation = generationRef.current + 1
    generationRef.current = generation
    const controller = new AbortController()

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
        setFailure(null)
        setSettledQuery(query)
      })
      .catch((requestError: unknown) => {
        if (controller.signal.aborted || generationRef.current !== generation) return
        setFailure({ query, message: getErrorMessage(requestError) })
        setSettledQuery(query)
      })

    return () => {
      controller.abort()
      if (generationRef.current === generation) {
        generationRef.current += 1
      }
    }
  }, [
    moduleFilter,
    statusFilter,
    keywordFilter,
    fromMs,
    toMs,
    page,
    pageSize,
    refreshVersion,
    query,
  ])

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
