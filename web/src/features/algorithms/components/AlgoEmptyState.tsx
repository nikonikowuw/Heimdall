import React from 'react'
import { Package, RotateCcw, Search, TriangleAlert, Upload } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { AlgoListState } from '../algoFilters'

export interface AlgoEmptyStateProps {
  state: Extract<AlgoListState, 'error' | 'empty' | 'no-match'>
  /** 服务端返回的错误详情；为空串表示未知错误，回退到 i18n 文案 */
  errorMessage?: string | null
  onRetry: () => void
  onUpload: () => void
  onClearFilters: () => void
}

/**
 * 列表缺失态。
 *
 * 「加载失败 / 仓库为空 / 筛选无命中」三态分开呈现：
 * 把接口故障渲染成「仓库暂无算法」会让运维去排查一个并不存在的问题，
 * 把筛选无命中渲染成空仓库又会让运维怀疑资产丢失。
 */
export function AlgoEmptyState({
  state,
  errorMessage,
  onRetry,
  onUpload,
  onClearFilters,
}: AlgoEmptyStateProps): React.ReactElement {
  const { t } = useTranslation('algo')

  const config = {
    error: {
      icon: TriangleAlert,
      iconClass: 'text-red-400',
      iconBgClass: 'bg-red-500/10',
      title: t('empty.errorTitle'),
      description: errorMessage || t('empty.errorDesc'),
      action: (
        <button
          type="button"
          onClick={onRetry}
          className="mt-5 flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95"
        >
          <RotateCcw className="h-4 w-4" />
          <span>{t('empty.retry')}</span>
        </button>
      ),
    },
    empty: {
      icon: Package,
      iconClass: 'text-[var(--accent)]',
      iconBgClass: 'bg-[var(--accent-soft)]',
      title: t('empty.emptyTitle'),
      description: t('empty.emptyDesc'),
      action: (
        <button
          type="button"
          onClick={onUpload}
          className="mt-5 flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95"
        >
          <Upload className="h-4 w-4" />
          <span>{t('empty.uploadAction')}</span>
        </button>
      ),
    },
    'no-match': {
      icon: Search,
      iconClass: 'text-[var(--text-muted)]',
      iconBgClass: 'bg-[var(--bg-secondary)]',
      title: t('empty.noMatchTitle'),
      description: t('empty.noMatchDesc'),
      action: (
        <button
          type="button"
          onClick={onClearFilters}
          className="mt-5 flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-4 py-2 text-xs font-semibold text-[var(--text-primary)] transition-all hover:border-[var(--border-strong)] active:scale-95"
        >
          <RotateCcw className="h-4 w-4" />
          <span>{t('empty.clearFilters')}</span>
        </button>
      ),
    },
  }[state]

  const Icon = config.icon

  return (
    <div className="frosted-glass flex flex-col items-center justify-center rounded-2xl border border-[var(--border)] px-6 py-16 text-center">
      <div
        className={`flex h-12 w-12 items-center justify-center rounded-2xl ${config.iconBgClass} ${config.iconClass}`}
      >
        <Icon className="h-6 w-6" />
      </div>
      <h3 className="mt-4 text-sm font-semibold text-[var(--text-primary)]">{config.title}</h3>
      <p className="mt-1 max-w-md text-xs leading-relaxed text-[var(--text-muted)]">
        {config.description}
      </p>
      {config.action}
    </div>
  )
}
