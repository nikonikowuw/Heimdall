import React from 'react'
import { Loader2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'

export interface AlgoListStatusProps {
  /** 服务端匹配总数 */
  total: number
  /** 已从服务端取回的条数 */
  loaded: number
  /** 当前实际渲染的条数（本地收敛后） */
  visible: number
  hasMore: boolean
  isLoadingMore?: boolean
  loadMoreError: string | null
  onLoadMore: () => void
}

/**
 * 清单计数与分页入口。
 *
 * 单页容量有上限，超出的部分必须显式告知并给出加载入口——
 * 「统计卡显示 130、网格只渲染 100」这种静默截断会让运维以为资产丢了。
 */
export function AlgoListStatus({
  total,
  loaded,
  visible,
  hasMore,
  isLoadingMore,
  loadMoreError,
  onLoadMore,
}: AlgoListStatusProps): React.ReactElement {
  const { t } = useTranslation('algo')

  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex min-w-0 flex-col gap-1">
        <p
          aria-live="polite"
          className="font-data text-[11px] text-[var(--text-muted)] tabular-nums"
        >
          {t('list.summary', { total, visible })}
          {hasMore && <span className="ml-2">{t('list.loadedOfTotal', { loaded, total })}</span>}
        </p>
        {loadMoreError !== null && (
          <p role="alert" className="text-[11px] text-[var(--accent-amber)]">
            {loadMoreError || t('list.loadMoreError')}
          </p>
        )}
      </div>

      {hasMore && (
        <button
          type="button"
          onClick={onLoadMore}
          disabled={isLoadingMore}
          className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3 py-1.5 text-xs font-medium text-[var(--text-primary)] transition-all hover:border-[var(--border-strong)] disabled:opacity-50"
        >
          {isLoadingMore && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
          <span>{t('list.loadMore')}</span>
        </button>
      )}
    </div>
  )
}
