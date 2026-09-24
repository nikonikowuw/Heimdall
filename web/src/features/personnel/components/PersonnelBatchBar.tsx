import React from 'react'
import { RefreshCw, Trash2, X, Users } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'

export interface PersonnelBatchBarProps {
  selectedCount: number
  isAllSelected: boolean
  isExecuting?: boolean
  onToggleSelectAll: () => void
  onClearSelection: () => void
  onBatchDelete: () => void
  onBatchReextract?: () => void
}

export function PersonnelBatchBar({
  selectedCount,
  isAllSelected,
  isExecuting = false,
  onToggleSelectAll,
  onClearSelection,
  onBatchDelete,
  onBatchReextract,
}: PersonnelBatchBarProps): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])
  const reduceMotion = useReducedMotion()

  return (
    <AnimatePresence>
      {selectedCount > 0 && (
        <motion.aside
          role="toolbar"
          aria-label={t('batch.selectedCount', { count: selectedCount })}
          initial={reduceMotion ? undefined : { opacity: 0, y: 32, scale: 0.96 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={reduceMotion ? undefined : { opacity: 0, y: 24, scale: 0.96 }}
          transition={{ duration: motionTokens.duration.normal, ease: motionTokens.easing.smooth }}
          className="fixed bottom-6 left-1/2 z-40 -translate-x-1/2"
        >
          <div className="lens-glass flex max-w-[calc(100vw-2rem)] items-center gap-2.5 overflow-x-auto rounded-2xl border border-[var(--border-strong)]/80 bg-[var(--bg-surface-solid)]/95 px-3.5 py-2 shadow-2xl backdrop-blur-xl sm:gap-3 sm:px-4 sm:py-2.5">
            {/* 勾选人员计数 */}
            <div className="flex shrink-0 items-center gap-2 border-r border-[var(--border)] pr-2.5 text-xs font-semibold text-[var(--text-primary)] sm:pr-3">
              <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-lg bg-[var(--accent)] text-white shadow-xs">
                <Users className="h-3.5 w-3.5" />
              </span>
              <span className="font-data whitespace-nowrap">
                {t('batch.selectedCount', { count: selectedCount })}
              </span>
            </div>

            {/* 当页全选/反选快捷切换 */}
            <button
              type="button"
              onClick={onToggleSelectAll}
              className="shrink-0 rounded-xl px-2 py-1 text-xs font-medium whitespace-nowrap text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:px-2.5"
            >
              {isAllSelected ? t('batch.clearSelection') : t('table.selectAll')}
            </button>

            {/* 批量重新提取特征 (可选) */}
            {onBatchReextract && (
              <button
                type="button"
                onClick={onBatchReextract}
                disabled={isExecuting}
                className="flex shrink-0 items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1.5 text-xs font-medium whitespace-nowrap text-[var(--text-secondary)] transition-all hover:border-[var(--border-strong)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50 sm:px-3"
              >
                <RefreshCw className={`h-3.5 w-3.5 ${isExecuting ? 'animate-spin' : ''}`} />
                <span>{t('batch.batchReextract')}</span>
              </button>
            )}

            {/* 批量删除 */}
            <button
              type="button"
              onClick={onBatchDelete}
              disabled={isExecuting}
              className="flex shrink-0 items-center gap-1.5 rounded-xl bg-[var(--status-danger-solid)] px-3 py-1.5 text-xs font-semibold whitespace-nowrap text-white shadow-xs transition-all hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--status-danger)]/50 focus-visible:outline-none active:scale-95 disabled:opacity-50 sm:px-3.5"
            >
              <Trash2 className="h-3.5 w-3.5" />
              <span>{t('batch.batchDelete')}</span>
            </button>

            {/* 取消勾选 */}
            <button
              type="button"
              onClick={onClearSelection}
              aria-label={t('batch.clearSelection')}
              title={t('batch.clearSelection')}
              className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </motion.aside>
      )}
    </AnimatePresence>
  )
}
