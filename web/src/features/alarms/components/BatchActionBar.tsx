import React from 'react'
import type { TFunction } from 'i18next'
import { CheckCircle2, RotateCcw, X } from 'lucide-react'

export interface BatchActionBarProps {
  selectedCount: number
  isProcessing: boolean
  onMarkProcessed: () => void
  onMarkUnprocessed: () => void
  onClearSelection: () => void
  t: TFunction
}

export function BatchActionBar({
  selectedCount,
  isProcessing,
  onMarkProcessed,
  onMarkUnprocessed,
  onClearSelection,
  t,
}: BatchActionBarProps): React.ReactElement | null {
  if (selectedCount === 0) return null

  return (
    <div className="animate-in slide-in-from-bottom-5 fixed bottom-6 left-1/2 z-40 flex -translate-x-1/2 items-center gap-3 rounded-2xl border border-[var(--border)] bg-[var(--bg-surface)]/95 px-5 py-3 shadow-2xl backdrop-blur-md duration-200">
      <div className="flex items-center gap-2 border-r border-[var(--border)] pr-3 text-xs font-semibold text-[var(--text-primary)]">
        <span className="flex h-5 w-5 items-center justify-center rounded-full bg-[var(--accent)] font-mono text-[11px] text-white">
          {selectedCount}
        </span>
        <span>{t('batch.selected', { count: selectedCount })}</span>
      </div>

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onMarkProcessed}
          disabled={isProcessing}
          className="flex items-center gap-1.5 rounded-xl border border-emerald-500/30 bg-emerald-500/15 px-3 py-1.5 text-xs font-semibold text-emerald-500 transition-all hover:bg-emerald-500 hover:text-white disabled:opacity-50"
        >
          <CheckCircle2 className="h-3.5 w-3.5" />
          <span>{t('batch.markProcessed')}</span>
        </button>

        <button
          type="button"
          onClick={onMarkUnprocessed}
          disabled={isProcessing}
          className="flex items-center gap-1.5 rounded-xl border border-amber-500/30 bg-amber-500/15 px-3 py-1.5 text-xs font-semibold text-amber-500 transition-all hover:bg-amber-500 hover:text-white disabled:opacity-50"
        >
          <RotateCcw className="h-3.5 w-3.5" />
          <span>{t('batch.markUnprocessed')}</span>
        </button>

        <button
          type="button"
          onClick={onClearSelection}
          disabled={isProcessing}
          className="flex items-center gap-1 rounded-xl px-2 py-1.5 text-xs text-[var(--text-muted)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
          title={t('batch.deselectAll')}
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    </div>
  )
}
