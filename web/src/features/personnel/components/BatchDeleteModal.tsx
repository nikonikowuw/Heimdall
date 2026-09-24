import React, { useId } from 'react'
import { AlertTriangle, Loader2, Trash2 } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { motionTokens } from '@/lib/motionTokens'
import type { PersonnelItem } from '@/types'

export interface BatchDeleteModalProps {
  isOpen: boolean
  targets: PersonnelItem[]
  isDeleting: boolean
  currentProgress?: { current: number; total: number } | null
  onClose: () => void
  onConfirm: () => void
}

export function BatchDeleteModal({
  isOpen,
  targets,
  isDeleting,
  currentProgress,
  onClose,
  onConfirm,
}: BatchDeleteModalProps): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])
  const reduceMotion = useReducedMotion()
  const titleId = useId()
  const descId = useId()

  useDismissStack(isOpen, onClose, { disabled: isDeleting })

  const count = targets.length

  return (
    <AnimatePresence>
      {isOpen && (
        <div className="modal-layer modal-layer--center">
          <motion.div
            aria-hidden="true"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: motionTokens.duration.fast }}
            onClick={isDeleting ? undefined : onClose}
            className="modal-scrim"
          />

          <motion.div
            role="alertdialog"
            aria-modal="true"
            aria-labelledby={titleId}
            aria-describedby={descId}
            initial={
              reduceMotion ? undefined : { opacity: 0, scale: 0.95, y: motionTokens.distance.sm }
            }
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={
              reduceMotion ? undefined : { opacity: 0, scale: 0.95, y: motionTokens.distance.sm }
            }
            transition={{
              duration: motionTokens.duration.normal,
              ease: motionTokens.easing.smooth,
            }}
            className="modal-surface modal-surface--compact modal-surface--glass border-[var(--status-danger-border)] p-6"
          >
            <div className="flex items-start gap-4">
              <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)] shadow-xs">
                <AlertTriangle className="h-5 w-5" />
              </div>

              <div className="min-w-0 flex-1">
                <h3 id={titleId} className="text-base font-bold text-[var(--text-primary)]">
                  {t('batch.deleteConfirmTitle')}
                </h3>
                <p id={descId} className="mt-1 text-xs leading-relaxed text-[var(--text-muted)]">
                  {t('batch.deleteConfirmDesc', { count })}
                </p>

                {/* 待删除人员预览清单 */}
                <div className="mt-3 max-h-36 overflow-y-auto rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/50 p-2 text-xs">
                  <div className="flex flex-wrap gap-1.5">
                    {targets.map((person) => (
                      <span
                        key={person.id}
                        className="inline-flex items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-0.5 text-[11px] font-medium text-[var(--text-primary)]"
                      >
                        <span>{person.name}</span>
                        <span className="font-data text-[10px] text-[var(--text-muted)]">
                          {`#${person.subjectId}`}
                        </span>
                      </span>
                    ))}
                  </div>
                </div>

                {/* 进度显示 */}
                {isDeleting && currentProgress && (
                  <div className="mt-3 text-xs text-[var(--status-danger)]">
                    {t('batch.deleting', {
                      current: currentProgress.current,
                      total: currentProgress.total,
                    })}
                  </div>
                )}
              </div>
            </div>

            <div className="mt-6 flex items-center justify-end gap-2.5">
              <button
                type="button"
                disabled={isDeleting}
                onClick={onClose}
                className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:border-[var(--border-strong)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
              >
                {t('actions.cancel')}
              </button>

              <button
                type="button"
                disabled={isDeleting}
                onClick={onConfirm}
                className="flex items-center gap-1.5 rounded-xl bg-[var(--status-danger-solid)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--status-danger)]/50 focus-visible:outline-none active:scale-95 disabled:opacity-50"
              >
                {isDeleting ? (
                  <>
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    <span>{t('actions.saving')}</span>
                  </>
                ) : (
                  <>
                    <Trash2 className="h-3.5 w-3.5" />
                    <span>{t('batch.batchDelete')}</span>
                  </>
                )}
              </button>
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}
