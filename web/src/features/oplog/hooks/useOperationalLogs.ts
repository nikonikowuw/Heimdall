import { useCallback, useEffect, useRef, useState } from 'react'
import type { EventFilter, LevelFilter, TargetFilter } from '@/features/oplog/logFilters'
import { DEFAULT_LOG_PAGE_SIZE } from '@/features/oplog/logPaging'
import { operationalLogApi } from '@/lib/api'
import type { OperationalLog } from '@/types'
import { getErrorMessage, normalizeOperationalLog } from './helpers'

export interface UseOperationalLogsFilters {
  /** 事件级别，`all` 表示不过滤 */
  level?: LevelFilter
  /** 归属模块，`all` 表示不过滤 */
  target?: TargetFilter
  /** 事件标记精确匹配，`all` 表示不过滤 */
  event?: EventFilter
  /** 关联摄像头 ID */
  cameraId?: string
  fromMs?: number
  toMs?: number
}

export interface UseOperationalLogsResult {
  /** 当前页记录；筛选与游标分页都在服务端完成，因此每页替换而非追加 */
  logs: OperationalLog[]
  isLoading: boolean
  hasMore: boolean
  error: string | null
  refresh: () => void
}

/**
 * 运维事件日志游标分页查询。
 *
 * 服务端以 `before` 毫秒游标向前翻页：游标历史按页码记录在 ref 中，只在回到第 1 页时
 * 清空重建；筛选条件下推服务端，前端不对已取回的一页做二次过滤。
 * 列表按页替换，单页容量受服务端 MAX_LIMIT(200) 与档位限制，天然低于 1000 条上限。
 */
export function useOperationalLogs(
  filters: UseOperationalLogsFilters,
  page: number = 1,
  pageSize: number = DEFAULT_LOG_PAGE_SIZE,
): UseOperationalLogsResult {
  const { level, target, event, cameraId, fromMs, toMs } = filters

  const [logs, setLogs] = useState<OperationalLog[]>([])
  const [isLoading, setIsLoading] = useState(true)
  const [hasMore, setHasMore] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [refreshVersion, setRefreshVersion] = useState(0)
  const generationRef = useRef(0)

  // 页码 -> 该页请求使用的 before 游标；第 1 页无游标
  const cursorMapRef = useRef<Map<number, number>>(new Map())
  // 生成游标的筛选范围：范围一变历史游标全部失效，必须从第 1 页重新建立
  const cursorScopeRef = useRef('')

  const levelFilter = level === undefined || level === 'all' ? undefined : level
  const targetFilter = target === undefined || target === 'all' ? undefined : target
  const eventFilter = event === undefined || event === 'all' ? undefined : event
  const trimmedCameraId = cameraId?.trim()
  const cameraFilter = trimmedCameraId ? trimmedCameraId : undefined

  const cursorScope = `${levelFilter ?? ''}|${targetFilter ?? ''}|${eventFilter ?? ''}|${
    cameraFilter ?? ''
  }|${fromMs ?? ''}|${toMs ?? ''}|${pageSize}`

  useEffect(() => {
    // 筛选范围或页容量变化时历史游标不再对应当前结果集，回到第 1 页由调用方负责
    if (cursorScopeRef.current !== cursorScope) {
      cursorScopeRef.current = cursorScope
      cursorMapRef.current.clear()
    }

    const generation = generationRef.current + 1
    generationRef.current = generation
    const controller = new AbortController()

    setIsLoading(true)
    setError(null)

    void operationalLogApi
      .list(
        {
          level: levelFilter,
          target: targetFilter,
          event: eventFilter,
          cameraId: cameraFilter,
          fromMs,
          toMs,
          limit: pageSize,
          before: page === 1 ? undefined : cursorMapRef.current.get(page),
        },
        controller.signal,
      )
      .then((pageData) => {
        if (generationRef.current !== generation) return
        setLogs(pageData.items.map(normalizeOperationalLog))
        setHasMore(pageData.hasMore)

        // 记录下一页所需的游标
        if (pageData.nextBefore !== null && pageData.nextBefore !== undefined) {
          cursorMapRef.current.set(page + 1, pageData.nextBefore)
        }
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
  }, [
    levelFilter,
    targetFilter,
    eventFilter,
    cameraFilter,
    fromMs,
    toMs,
    page,
    pageSize,
    cursorScope,
    refreshVersion,
  ])

  // 保留游标历史，刷新当前页仍取同一游标，避免翻页后刷新退化成第 1 页数据
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
