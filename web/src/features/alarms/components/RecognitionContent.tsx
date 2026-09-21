import React from 'react'
import { RotateCcw, Search, UserCheck } from 'lucide-react'
import type { RecognitionRecord } from '@/types'
import type { ViewMode } from './AlarmsContent'
import { RecognitionCardItem } from './RecognitionCardItem'
import { RecognitionTableRow } from './RecognitionTableRow'

export interface RecognitionContentProps {
  recognitions: RecognitionRecord[]
  viewMode?: ViewMode
  cameraNameMap?: Record<string, string>
  hasActiveFilters?: boolean
  searchQuery?: string
  onResetFilters?: () => void
  onClearSearch?: () => void
  onOpenReview: (rec: RecognitionRecord) => void
  onQuickReview: (rec: RecognitionRecord, status: 'confirmed' | 'rejected') => void
  t: (key: string, options?: Record<string, unknown>) => string
}

export function RecognitionContent({
  recognitions,
  viewMode = 'cards',
  cameraNameMap,
  hasActiveFilters = false,
  searchQuery,
  onResetFilters,
  onClearSearch,
  onOpenReview,
  onQuickReview,
  t,
}: RecognitionContentProps): React.ReactElement {
  if (recognitions.length === 0) {
    if (searchQuery && searchQuery.trim()) {
      return (
        <div className="flex flex-col items-center justify-center py-24 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-2xl border border-[var(--status-success-border)] bg-[var(--status-success-soft)] text-[var(--status-success)] shadow-xs">
            <Search className="h-6 w-6" />
          </div>
          <p className="mt-3 text-sm font-semibold tracking-tight text-[var(--text-primary)]">
            {t('search.noMatch', { query: searchQuery.trim() })}
          </p>
          {onClearSearch && (
            <button
              type="button"
              onClick={onClearSearch}
              className="mt-4 flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-1.5 text-xs font-medium text-[var(--accent)] shadow-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)]/20 active:scale-95"
            >
              <RotateCcw className="h-3.5 w-3.5" />
              <span>{t('search.clearQuery')}</span>
            </button>
          )}
        </div>
      )
    }

    return (
      <div className="flex flex-col items-center justify-center py-24 text-center">
        <div className="flex h-12 w-12 items-center justify-center rounded-2xl border border-[var(--status-success-border)] bg-[var(--status-success-soft)] text-[var(--status-success)] shadow-xs">
          <UserCheck className="h-6 w-6" />
        </div>
        <p className="mt-3 text-sm font-semibold tracking-tight text-[var(--text-primary)]">
          {t('empty.recognitions')}
        </p>
        <p className="mt-1 text-xs text-[var(--text-muted)]">{t('empty.recognitionsDesc')}</p>
        {hasActiveFilters && onResetFilters && (
          <button
            type="button"
            onClick={onResetFilters}
            className="mt-4 flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-1.5 text-xs font-medium text-[var(--accent)] shadow-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)]/20 active:scale-95"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            <span>{t('empty.resetFilter')}</span>
          </button>
        )}
      </div>
    )
  }

  if (viewMode === 'table') {
    return (
      <div className="w-full overflow-x-auto">
        <table className="w-full text-left text-xs text-[var(--text-secondary)]">
          <thead className="sticky top-0 z-10 border-b border-[var(--border)]/70 bg-[var(--bg-secondary)]/80 text-[11px] font-semibold tracking-wider text-[var(--text-muted)] uppercase backdrop-blur-md">
            <tr>
              <th className="px-3.5 py-3">{t('columns.status')}</th>
              <th className="px-3.5 py-3">{t('columns.fieldCapture')}</th>
              <th className="px-3.5 py-3">{t('columns.similarity')}</th>
              <th className="px-3.5 py-3">{t('columns.archivePhoto')}</th>
              <th className="px-3.5 py-3">{t('columns.subject')}</th>
              <th className="px-3.5 py-3">{t('columns.camera')}</th>
              <th className="px-3.5 py-3">{t('columns.occurredAt')}</th>
              <th className="px-3.5 py-3">{t('columns.sitePanorama')}</th>
              <th className="px-3.5 py-3 text-right">{t('columns.actions')}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-[var(--border)]/40">
            {recognitions.map((recognition) => (
              <RecognitionTableRow
                key={recognition.recognitionId || recognition.id}
                recognition={recognition}
                cameraName={cameraNameMap?.[recognition.cameraId]}
                onOpenReview={onOpenReview}
                onQuickReview={onQuickReview}
                t={t}
              />
            ))}
          </tbody>
        </table>
      </div>
    )
  }

  return (
    <div className="grid grid-cols-1 gap-5 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-3">
      {recognitions.map((recognition) => (
        <RecognitionCardItem
          key={recognition.recognitionId || recognition.id}
          recognition={recognition}
          cameraName={cameraNameMap?.[recognition.cameraId]}
          onOpenReview={onOpenReview}
          onQuickReview={onQuickReview}
          t={t}
        />
      ))}
    </div>
  )
}
