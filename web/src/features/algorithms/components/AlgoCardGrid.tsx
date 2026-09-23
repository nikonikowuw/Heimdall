import React from 'react'
import { AnimatePresence } from 'motion/react'
import type { AlgorithmItem } from '@/types'
import type { AlgoListState } from '../algoFilters'
import type { AlgoUsageEntry } from '../algoUsage'
import { AlgoCard } from './AlgoCard'
import { AlgoEmptyState } from './AlgoEmptyState'

export interface AlgoCardGridProps {
  algorithms: AlgorithmItem[]
  state: AlgoListState
  /** 服务端错误详情；为空串表示未知错误 */
  errorMessage?: string | null
  /** 算法 ID → 占用通道任务 */
  usageByAlgorithm: ReadonlyMap<string, AlgoUsageEntry[]>
  onManageVersions: (algo: AlgorithmItem) => void
  onViewSchema: (algo: AlgorithmItem) => void
  onRetry: () => void
  onUpload: () => void
  onClearFilters: () => void
}

export function AlgoCardGrid({
  algorithms,
  state,
  errorMessage,
  usageByAlgorithm,
  onManageVersions,
  onViewSchema,
  onRetry,
  onUpload,
  onClearFilters,
}: AlgoCardGridProps): React.ReactElement {
  // 骨架仅在确实无内容可展示时出现（deriveAlgoListState 保证 loading 态下列表为空），
  // 已有卡片时改由计数行与刷新按钮表达在途状态，不把卡片抹掉重画
  if (state === 'loading') {
    return (
      <div
        aria-busy="true"
        className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4"
      >
        {[1, 2, 3, 4].map((n) => (
          <div
            key={n}
            // 骨架屏保留 animate-pulse（加载语义），但不叠 .frosted-glass：
            // 它自己既做 backdrop 采样又做逐帧动画会让模糊结果每帧重算；
            // 骨架屏使用不透明底色，模糊没有视觉收益。
            className="h-44 animate-pulse rounded-2xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-5"
          />
        ))}
      </div>
    )
  }

  if (state !== 'ready') {
    return (
      <AlgoEmptyState
        state={state}
        errorMessage={errorMessage}
        onRetry={onRetry}
        onUpload={onUpload}
        onClearFilters={onClearFilters}
      />
    )
  }

  return (
    <div
      role="list"
      className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4"
    >
      <AnimatePresence mode="popLayout">
        {algorithms.map((algo) => (
          <AlgoCard
            key={algo.algorithmId}
            algorithm={algo}
            usage={usageByAlgorithm.get(algo.algorithmId) ?? []}
            onManageVersions={onManageVersions}
            onViewSchema={onViewSchema}
          />
        ))}
      </AnimatePresence>
    </div>
  )
}
