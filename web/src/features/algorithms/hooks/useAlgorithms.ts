import { useCallback, useEffect, useRef, useState } from 'react'
import { useDebounce } from '@/hooks/use-debounce'
import { algorithmApi } from '@/lib/api'
import type { AlgorithmItem, AlgorithmStats, HostPlatformInfo } from '@/types'
import { ALGO_LIST_PAGE_SIZE, mergeAlgorithmPages, type AlgoOriginFilter } from '../algoFilters'

/** 搜索输入防抖窗口，与 oplog 保持同一档位 */
const SEARCH_DEBOUNCE_MS = 250

export interface UseAlgorithmsParams {
  /** 输入框即时关键字；请求侧自行防抖，调用方直接传最新输入值 */
  keyword: string
  /** 算法类型；`all` 不做过滤 */
  algorithmType: string
  origin: AlgoOriginFilter
  /** 外部刷新信号：递增即重新拉取列表、统计与宿主平台 */
  refreshVersion: number
}

export interface UseAlgorithmsResult {
  /** 已取回的算法（分页追加后按 ID 去重） */
  algorithms: AlgorithmItem[]
  /** 服务端匹配总数，用于区分截断、空仓库与筛选无命中 */
  total: number
  isLoading: boolean
  isLoadingMore: boolean
  /** 追加分页失败时保留已有列表，并允许用户重试 */
  loadMoreError: string | null
  /** 列表请求错误；为空串表示未知错误，由视图回退到 i18n 文案 */
  listError: string | null
  stats: AlgorithmStats | null
  isLoadingStats: boolean
  statsError: string | null
  hostPlatform: HostPlatformInfo | null
  hasMore: boolean
  loadMore: () => void
}

function toErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message
  return ''
}

/**
 * 算法仓库数据源。
 *
 * 三个关注点刻意分开：
 * 1. **列表**随 `keyword`（防抖）/`algorithmType`/`origin` 变化重新拉取，筛选下推服务端，
 *    前端不对服务端结果做二次收紧判断；
 * 2. **统计**与筛选无关，只在挂载和显式刷新时取一次，避免输入关键字时统计数字被反复置空；
 * 3. **宿主平台**由后端给出（编译期平台 + 后端 feature），前端不做任何环境嗅探。
 *
 * 所有在途请求带 AbortController 与代际号：快速输入时旧响应不会覆盖新结果。
 */
export function useAlgorithms(params: UseAlgorithmsParams): UseAlgorithmsResult {
  const { keyword, algorithmType, origin, refreshVersion } = params

  const debouncedKeyword = useDebounce(keyword.trim(), SEARCH_DEBOUNCE_MS)

  const [algorithms, setAlgorithms] = useState<AlgorithmItem[]>([])
  const [total, setTotal] = useState(0)
  const [isLoading, setIsLoading] = useState(true)
  const [isLoadingMore, setIsLoadingMore] = useState(false)
  const [loadMoreError, setLoadMoreError] = useState<string | null>(null)
  const [listError, setListError] = useState<string | null>(null)

  const [stats, setStats] = useState<AlgorithmStats | null>(null)
  const [isLoadingStats, setIsLoadingStats] = useState(true)
  const [statsError, setStatsError] = useState<string | null>(null)

  const [hostPlatform, setHostPlatform] = useState<HostPlatformInfo | null>(null)
  const [page, setPage] = useState(1)

  // 当前结果集的作用域标识：加载更多只允许追加到同一作用域，跨作用域一律丢弃
  const scope = `${debouncedKeyword}|${algorithmType}|${origin}`
  const scopeRef = useRef(scope)
  // 列表与统计各自维护代际号：两者生命周期不同，共用计数器会让先到的响应被后发的请求误判为过期
  const listGenerationRef = useRef(0)
  const statsGenerationRef = useRef(0)
  const loadMoreControllerRef = useRef<AbortController | null>(null)

  let isBuiltin: boolean | undefined
  if (origin === 'builtin') {
    isBuiltin = true
  } else if (origin === 'custom') {
    isBuiltin = false
  }

  useEffect(() => {
    scopeRef.current = scope
    loadMoreControllerRef.current?.abort()
    loadMoreControllerRef.current = null
    setIsLoadingMore(false)
    setLoadMoreError(null)

    const generation = listGenerationRef.current + 1
    listGenerationRef.current = generation
    const controller = new AbortController()

    setIsLoading(true)
    setListError(null)

    void algorithmApi
      .list(
        {
          page: 1,
          pageSize: ALGO_LIST_PAGE_SIZE,
          keyword: debouncedKeyword || undefined,
          algorithmType: algorithmType !== 'all' ? algorithmType : undefined,
          isBuiltin,
        },
        controller.signal,
      )
      .then((data) => {
        if (controller.signal.aborted || listGenerationRef.current !== generation) return
        setAlgorithms(data.items)
        setTotal(data.total)
        setPage(1)
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted || listGenerationRef.current !== generation) return
        setListError(toErrorMessage(error))
      })
      .finally(() => {
        if (listGenerationRef.current === generation) setIsLoading(false)
      })

    return () => {
      controller.abort()
      loadMoreControllerRef.current?.abort()
      loadMoreControllerRef.current = null
      if (listGenerationRef.current === generation) listGenerationRef.current += 1
    }
  }, [debouncedKeyword, algorithmType, isBuiltin, refreshVersion, scope])

  // 统计与宿主平台与筛选无关，不随关键字变化重取
  useEffect(() => {
    const generation = statsGenerationRef.current + 1
    statsGenerationRef.current = generation
    const controller = new AbortController()

    setIsLoadingStats(true)
    setStatsError(null)

    void Promise.all([
      algorithmApi.getStats(),
      algorithmApi.getHostPlatform(controller.signal).catch(() => null),
    ])
      .then(([statsData, hostData]) => {
        if (controller.signal.aborted || statsGenerationRef.current !== generation) return
        setStats(statsData)
        setStatsError(null)
        if (hostData) setHostPlatform(hostData)
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted || statsGenerationRef.current !== generation) return
        setStatsError(toErrorMessage(error))
      })
      .finally(() => {
        if (statsGenerationRef.current === generation) setIsLoadingStats(false)
      })

    return () => {
      controller.abort()
    }
  }, [refreshVersion])

  const hasMore = page * ALGO_LIST_PAGE_SIZE < total

  const loadMore = useCallback(() => {
    if (isLoading || isLoadingMore || !hasMore) return

    const targetPage = page + 1
    const requestScope = scopeRef.current
    loadMoreControllerRef.current?.abort()
    const controller = new AbortController()
    loadMoreControllerRef.current = controller
    setIsLoadingMore(true)
    setLoadMoreError(null)

    void algorithmApi
      .list(
        {
          page: targetPage,
          pageSize: ALGO_LIST_PAGE_SIZE,
          keyword: debouncedKeyword || undefined,
          algorithmType: algorithmType !== 'all' ? algorithmType : undefined,
          isBuiltin,
        },
        controller.signal,
      )
      .then((data) => {
        // 作用域已变（用户改了筛选）或请求被取代时丢弃，避免把结果追加进另一份清单
        if (controller.signal.aborted || scopeRef.current !== requestScope) return
        setAlgorithms((current) => mergeAlgorithmPages(current, data.items))
        setTotal(data.total)
        setPage(targetPage)
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted || scopeRef.current !== requestScope) return
        setLoadMoreError(toErrorMessage(error))
      })
      .finally(() => {
        if (loadMoreControllerRef.current === controller) {
          loadMoreControllerRef.current = null
          setIsLoadingMore(false)
        }
      })
  }, [algorithmType, debouncedKeyword, hasMore, isBuiltin, isLoading, isLoadingMore, page])

  return {
    algorithms,
    total,
    isLoading,
    isLoadingMore,
    loadMoreError,
    listError,
    stats,
    isLoadingStats,
    statsError,
    hostPlatform,
    hasMore,
    loadMore,
  }
}
