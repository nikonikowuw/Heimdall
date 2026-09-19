import type { ReactElement } from 'react'
import { RotateCcw, SearchX } from 'lucide-react'
import { useTranslation } from 'react-i18next'

interface LogEmptyStateProps {
  isFiltered?: boolean
  onClearFilters?: () => void
}

export function LogEmptyState({
  isFiltered = false,
  onClearFilters,
}: LogEmptyStateProps): ReactElement {
  const { t } = useTranslation('oplog')

  return (
    <div className="flex min-h-64 flex-col items-center justify-center p-8 text-center">
      <div className="relative mb-3 flex h-14 w-14 items-center justify-center rounded-2xl border border-[var(--border)] bg-[var(--bg-surface-solid)] shadow-xs">
        <div className="absolute inset-0 rounded-2xl bg-[var(--accent-soft)] opacity-50" />
        <SearchX className="relative z-10 h-6 w-6 text-[var(--text-muted)]" strokeWidth={1.5} />
      </div>

      <h3 className="text-sm font-semibold text-[var(--text-primary)]">
        {isFiltered ? t('emptyState.noResultsTitle') : t('empty')}
      </h3>

      <p className="mt-1 max-w-sm text-xs leading-relaxed text-[var(--text-muted)]">
        {isFiltered ? t('emptyState.noResultsDesc') : t('empty')}
      </p>

      {isFiltered && onClearFilters && (
        <button
          type="button"
          onClick={onClearFilters}
          className="reticle-target mt-4 inline-flex items-center gap-1.5 rounded-lg border border-[var(--border-strong)] bg-[var(--bg-surface-solid)] px-3.5 py-1.5 text-xs font-medium text-[var(--text-primary)] shadow-xs transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)]"
        >
          <RotateCcw className="h-3.5 w-3.5" />
          <span>{t('emptyState.clearAll')}</span>
        </button>
      )}
    </div>
  )
}
