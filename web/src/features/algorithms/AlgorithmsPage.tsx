import React, { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { AlgorithmItem } from '@/types'
import {
  collectAlgorithmTypes,
  collectPlatformOptions,
  COMMON_ALGORITHM_TYPES,
  deriveAlgoListState,
  filterAlgorithms,
  hasActiveFilters,
  sortAlgorithms,
  withSelectedOption,
  type AlgoListQuery,
  type AlgoOriginFilter,
  type AlgoSortKey,
} from './algoFilters'
import { AlgoCardGrid } from './components/AlgoCardGrid'
import { AlgoFilterBar } from './components/AlgoFilterBar'
import { AlgoListStatus } from './components/AlgoListStatus'
import { AlgoStatsHeader } from './components/AlgoStatsHeader'
import { HostPlatformBadge } from './components/HostPlatformBadge'
import { SchemaModal } from './components/SchemaModal'
import { UploadModal } from './components/UploadModal'
import { VersionsDrawer } from './components/VersionsDrawer'
import { useAlgorithms } from './hooks/useAlgorithms'
import { useAlgoUsage } from './hooks/useAlgoUsage'

export interface AlgorithmsPageProps {
  /** 从占用提示跳转到任务布防页并定位指定通道 */
  onNavigateToTask?: (cameraId: string) => void
}

/**
 * 算法仓库。
 *
 * 页面只做三件事：持有筛选状态、把服务端结果与本地收敛结果拼起来、分发浮层。
 * 数据获取（防抖、取消、分页）在 `useAlgorithms`，占用关系在 `useAlgoUsage`，
 * 筛选与状态派生在 `algoFilters`（纯函数，单独测试）。
 */
export function AlgorithmsPage({ onNavigateToTask }: AlgorithmsPageProps): React.ReactElement {
  const { t } = useTranslation('algo')

  // 输入框即时值用于本地即时反馈，请求侧由 hook 自行防抖
  const [keyword, setKeyword] = useState('')
  const [algorithmType, setAlgorithmType] = useState('all')
  const [origin, setOrigin] = useState<AlgoOriginFilter>('all')
  const [platform, setPlatform] = useState('all')
  const [sortKey, setSortKey] = useState<AlgoSortKey>('default')
  const [refreshVersion, setRefreshVersion] = useState(0)

  const [selectedAlgoIdForVersions, setSelectedAlgoIdForVersions] = useState<string | null>(null)
  const [selectedAlgoIdForSchema, setSelectedAlgoIdForSchema] = useState<string | null>(null)
  const [isUploadModalOpen, setIsUploadModalOpen] = useState(false)

  const {
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
  } = useAlgorithms({ keyword, algorithmType, origin, refreshVersion })

  const {
    usage: usageByAlgorithm,
    isLoading: isLoadingUsage,
    hasError: usageError,
  } = useAlgoUsage(refreshVersion)

  const query: AlgoListQuery = useMemo(
    () => ({ keyword, algorithmType, origin, platform, sortKey }),
    [keyword, algorithmType, origin, platform, sortKey],
  )

  // 选项与选中值合并：零命中时选项表不能丢掉正在生效的条件，否则控件显示会与状态不符
  const typeOptions = useMemo(
    () =>
      withSelectedOption(
        [...new Set([...COMMON_ALGORITHM_TYPES, ...collectAlgorithmTypes(algorithms)])],
        algorithmType,
      ),
    [algorithms, algorithmType],
  )
  const platformOptions = useMemo(
    () => withSelectedOption(collectPlatformOptions(algorithms), platform),
    [algorithms, platform],
  )

  // 平台与排序是纯前端维度；关键字/类型/来源服务端已下推，这里只做输入期的即时收敛
  const visibleAlgorithms = useMemo(
    () => sortAlgorithms(filterAlgorithms(algorithms, query), sortKey),
    [algorithms, query, sortKey],
  )

  const isResolvingPlatformFilter =
    platform !== 'all' &&
    visibleAlgorithms.length === 0 &&
    hasMore &&
    !loadMoreError &&
    listError === null

  const listState = deriveAlgoListState({
    hasError: listError !== null,
    isLoading: isLoading || isResolvingPlatformFilter,
    totalCount: total,
    visibleCount: visibleAlgorithms.length,
  })

  useEffect(() => {
    if (
      platform === 'all' ||
      visibleAlgorithms.length > 0 ||
      !hasMore ||
      isLoading ||
      isLoadingMore ||
      loadMoreError ||
      listError
    ) {
      return
    }
    loadMore()
  }, [
    hasMore,
    isLoading,
    isLoadingMore,
    loadMore,
    loadMoreError,
    listError,
    platform,
    visibleAlgorithms.length,
  ])

  const selectedAlgoForVersions = useMemo(
    () => algorithms.find((algo) => algo.algorithmId === selectedAlgoIdForVersions) ?? null,
    [algorithms, selectedAlgoIdForVersions],
  )
  const selectedAlgoForSchema = useMemo(
    () => algorithms.find((algo) => algo.algorithmId === selectedAlgoIdForSchema) ?? null,
    [algorithms, selectedAlgoIdForSchema],
  )

  const handleRefresh = useCallback(() => setRefreshVersion((version) => version + 1), [])

  const handleClearFilters = useCallback(() => {
    setKeyword('')
    setAlgorithmType('all')
    setOrigin('all')
    setPlatform('all')
  }, [])

  const handleQueryChange = useCallback((patch: Partial<AlgoListQuery>) => {
    if (patch.keyword !== undefined) setKeyword(patch.keyword)
    if (patch.algorithmType !== undefined) setAlgorithmType(patch.algorithmType)
    if (patch.origin !== undefined) setOrigin(patch.origin)
    if (patch.platform !== undefined) setPlatform(patch.platform)
    if (patch.sortKey !== undefined) setSortKey(patch.sortKey)
  }, [])

  const handleManageVersions = useCallback((algo: AlgorithmItem) => {
    setSelectedAlgoIdForVersions(algo.algorithmId)
  }, [])

  const handleViewSchema = useCallback((algo: AlgorithmItem) => {
    setSelectedAlgoIdForSchema(algo.algorithmId)
  }, [])

  const handleNavigateToTask = useCallback(
    (cameraId: string) => {
      onNavigateToTask?.(cameraId)
    },
    [onNavigateToTask],
  )

  // 抽屉打开期间清单可能因刷新而变；目标算法消失时抽屉随之关闭（不保留幽灵数据）
  const drawerUsage = selectedAlgoForVersions
    ? (usageByAlgorithm.get(selectedAlgoForVersions.algorithmId) ?? [])
    : []

  return (
    <div className="flex h-full w-full flex-col gap-5 overflow-y-auto pr-1 text-[var(--text-primary)]">
      {/* 顶部标题与宿主平台：设备维度只出现一次，不占用统计网格 */}
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="space-y-1">
          <h1 className="text-xl font-bold tracking-tight text-[var(--text-primary)]">
            {t('title')}
          </h1>
          <p className="text-xs text-[var(--text-muted)]">{t('subtitle')}</p>
        </div>
        <HostPlatformBadge platform={hostPlatform} />
      </div>

      <AlgoStatsHeader stats={stats} isLoading={isLoadingStats} error={statsError} />

      <div className="space-y-3">
        <AlgoFilterBar
          query={query}
          onQueryChange={handleQueryChange}
          typeOptions={typeOptions}
          platformOptions={platformOptions}
          hasActiveFilters={hasActiveFilters(query)}
          onClearFilters={handleClearFilters}
          onRefresh={handleRefresh}
          onOpenUpload={() => setIsUploadModalOpen(true)}
          isLoading={isLoading}
        />

        {/* 内容为空时计数行交给缺失态表达，避免「共 0 项」与空态重复 */}
        {listState === 'ready' || (visibleAlgorithms.length === 0 && hasMore) ? (
          <AlgoListStatus
            total={total}
            loaded={algorithms.length}
            visible={visibleAlgorithms.length}
            hasMore={hasMore}
            isLoadingMore={isLoadingMore}
            loadMoreError={loadMoreError}
            onLoadMore={loadMore}
          />
        ) : null}

        {/* 刷新失败但仍有旧数据：以横幅提示，不清空可用清单 */}
        {listError !== null && listState === 'ready' && (
          <p role="status" className="text-[11px] text-[var(--accent-amber)]">
            {listError || t('empty.errorDesc')}
          </p>
        )}

        <AlgoCardGrid
          algorithms={visibleAlgorithms}
          state={listState}
          errorMessage={listError}
          usageByAlgorithm={usageByAlgorithm}
          onManageVersions={handleManageVersions}
          onViewSchema={handleViewSchema}
          onRetry={handleRefresh}
          onUpload={() => setIsUploadModalOpen(true)}
          onClearFilters={handleClearFilters}
        />
      </div>

      <VersionsDrawer
        isOpen={Boolean(selectedAlgoForVersions)}
        algorithm={selectedAlgoForVersions}
        usage={drawerUsage}
        usageLoading={isLoadingUsage}
        usageError={usageError}
        onClose={() => setSelectedAlgoIdForVersions(null)}
        onRefresh={handleRefresh}
        onNavigateToTask={onNavigateToTask ? handleNavigateToTask : undefined}
      />

      <UploadModal
        isOpen={isUploadModalOpen}
        onClose={() => setIsUploadModalOpen(false)}
        onSuccess={handleRefresh}
      />

      <SchemaModal
        isOpen={Boolean(selectedAlgoForSchema)}
        algorithm={selectedAlgoForSchema}
        onClose={() => setSelectedAlgoIdForSchema(null)}
      />
    </div>
  )
}
