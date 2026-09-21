import React from 'react'
import type { TFunction } from 'i18next'
import { CheckCircle2, RotateCcw, X } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { motionTokens } from '@/lib/motionTokens'

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
}: BatchActionBarProps): React.ReactElement {
  const shouldReduce = useReducedMotion()

  return (
    <AnimatePresence>
      {selectedCount > 0 && (
        <motion.div
          initial={{ opacity: 0, y: shouldReduce ? 0 : 20, x: '-50%' }}
          animate={{ opacity: 1, y: 0, x: '-50%' }}
          exit={{ opacity: 0, y: shouldReduce ? 0 : 20, x: '-50%' }}
          transition={{
            duration: shouldReduce ? 0.1 : motionTokens.duration.fast,
            ease: motionTokens.easing.smooth,
          }}
          className="fixed bottom-6 left-1/2 z-40 flex items-center gap-3 rounded-2xl border border-[var(--border)] bg-[var(--bg-surface)]/95 px-5 py-3 shadow-2xl backdrop-blur-md"
        >
          <div className="flex items-center gap-2 border-r border-[var(--border)] pr-3 text-xs font-semibold text-[var(--text-primary)]">
            <span className="flex h-5 w-5 items-center justify-center rounded-full bg-[var(--accent)] font-mono text-[11px] text-white">
              {selectedCount}
            </span>
            <span>{t('batch.selected', { count: selectedCount })}</span>
          </div>

          <div className="flex items-center gap-2">
            <motion.button
              type="button"
              whileTap={{ scale: 0.96 }}
              onClick={onMarkProcessed}
              disabled={isProcessing}
              className="flex items-center gap-1.5 rounded-xl border border-[var(--status-success-border)] bg-[var(--status-success-soft)] px-3 py-1.5 text-xs font-semibold text-[var(--status-success)] transition-all hover:bg-[var(--status-success)] hover:text-white disabled:opacity-50"
            >
              <CheckCircle2 className="h-3.5 w-3.5" />
              <span>{t('batch.markProcessed')}</span>
            </motion.button>

            <motion.button
              type="button"
              whileTap={{ scale: 0.96 }}
              onClick={onMarkUnprocessed}
              disabled={isProcessing}
              className="flex items-center gap-1.5 rounded-xl border border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] px-3 py-1.5 text-xs font-semibold text-[var(--status-warning)] transition-all hover:bg-[var(--status-warning)] hover:text-white disabled:opacity-50"
            >
              <RotateCcw className="h-3.5 w-3.5" />
              <span>{t('batch.markUnprocessed')}</span>
            </motion.button>

            <button
              type="button"
              onClick={onClearSelection}
              className="rounded-xl p-1.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
              title={t('batch.deselectAll')}
              aria-label={t('batch.deselectAll')}
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}
