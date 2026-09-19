import type { ReactElement } from 'react'
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { LOG_PAGE_SIZE_OPTIONS } from '../logPaging'

export interface LogPaginationBarProps {
  page: number
  pageSize: number
  pageSizeOptions?: readonly number[]
  onPageChange: (newPage: number) => void
  onPageSizeChange: (newSize: number) => void
  hasMore: boolean
  isLoading: boolean
  totalOnCurrentPage: number
}

export function LogPaginationBar({
  page,
  pageSize,
  pageSizeOptions = LOG_PAGE_SIZE_OPTIONS,
  onPageChange,
  onPageSizeChange,
  hasMore,
  isLoading,
  totalOnCurrentPage,
}: LogPaginationBarProps): ReactElement {
  const { t } = useTranslation('oplog')

  const hasPrev = page > 1

  return (
    <footer className="frosted-glass flex flex-wrap items-center justify-between gap-3 rounded-xl px-4 py-2.5 shadow-2xs">
      {/* 左侧：当前页条数与每页容量选择 */}
      <div className="flex items-center gap-3">
        <span className="font-data text-xs text-[var(--text-muted)]">
          {t('pagination.currentCount', { count: totalOnCurrentPage })}
        </span>

        <div className="flex items-center gap-1.5">
          <label htmlFor="log-page-size" className="sr-only">
            {t('pagination.pageSize', { size: pageSize })}
          </label>
          <select
            id="log-page-size"
            value={pageSize}
            onChange={(e) => onPageSizeChange(Number(e.target.value))}
            disabled={isLoading}
            className="h-7 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] px-2 text-xs font-medium text-[var(--text-secondary)] transition-colors outline-none focus:border-[var(--accent)] disabled:opacity-50"
          >
            {pageSizeOptions.map((opt) => (
              <option key={opt} value={opt}>
                {t('pagination.pageSize', { size: opt })}
              </option>
            ))}
          </select>
        </div>
      </div>

      {/* 右侧：上一页 / 页码指示 / 下一页 */}
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => onPageChange(page - 1)}
          disabled={!hasPrev || isLoading}
          className="reticle-target flex h-7 items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] px-2.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-40"
          aria-label={t('pagination.prev')}
        >
          <ChevronLeft className="h-3.5 w-3.5" />
          <span className="hidden sm:inline">{t('pagination.prev')}</span>
        </button>

        <div className="flex h-7 items-center justify-center rounded-lg border border-[var(--accent)]/30 bg-[var(--accent-soft)] px-3 text-xs font-semibold text-[var(--accent)]">
          <span>{t('pagination.page', { current: page })}</span>
        </div>

        <button
          type="button"
          onClick={() => onPageChange(page + 1)}
          disabled={!hasMore || isLoading}
          className="reticle-target flex h-7 items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] px-2.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-40"
          aria-label={t('pagination.next')}
        >
          <span className="hidden sm:inline">{t('pagination.next')}</span>
          <ChevronRight className="h-3.5 w-3.5" />
        </button>
      </div>
    </footer>
  )
}
