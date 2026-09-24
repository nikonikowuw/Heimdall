import { useId } from 'react'
import { AlertTriangle, Loader2, Trash2, User, X } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { evidenceApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import type { PersonnelItem } from '@/types'

export interface DeleteConfirmModalProps {
  isOpen: boolean
  target: PersonnelItem | null
  isDeleting: boolean
  onClose: () => void
  onConfirm: () => void
}

/**
 * 人员物理删除二次确认。
 *
 * 确认对象必须可被肉眼核对（头像 + 姓名 + 编号），因为删除会同时销毁数据库记录、
 * 人脸特征向量与本地照片文件；仅靠一句「确定删除吗」不足以避免误删同名人。
 */
export function DeleteConfirmModal({
  isOpen,
  target,
  isDeleting,
  onClose,
  onConfirm,
}: DeleteConfirmModalProps) {
  const { t } = useTranslation(['personnel', 'common'])
  const reduceMotion = useReducedMotion()
  const titleId = useId()

  // ESC 浮层栈支持；Enter 经同一栈分发，仅栈顶弹窗可接管确认
  useDismissStack(isOpen && Boolean(target), onClose, { disabled: isDeleting, onConfirm })

  const avatarUrl = target?.primaryPhotoPath ? evidenceApi.getImageUrl(target.primaryPhotoPath) : ''

  return (
    <AnimatePresence>
      {isOpen && target && (
        <div
          onClick={(e) => {
            if (e.target === e.currentTarget && !isDeleting) onClose()
          }}
          className="modal-backdrop modal-backdrop--raised"
        >
          <motion.div
            role="alertdialog"
            aria-modal="true"
            aria-labelledby={titleId}
            initial={reduceMotion ? false : { opacity: 0, scale: 0.96, y: 12 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={reduceMotion ? { opacity: 0 } : { opacity: 0, scale: 0.96, y: 12 }}
            transition={{
              duration: motionTokens.duration.normal,
              ease: motionTokens.easing.smooth,
            }}
            className="modal-surface modal-surface--compact modal-surface--glass"
          >
            {/* 头部 */}
            <div className="flex items-start justify-between gap-3 border-b border-[var(--border)]/70 px-5 py-4">
              <div className="flex min-w-0 items-center gap-3.5">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)] shadow-xs">
                  <Trash2 className="h-5 w-5" aria-hidden="true" />
                </div>
                <div className="min-w-0">
                  <h3
                    id={titleId}
                    className="truncate text-base font-bold tracking-tight text-[var(--text-primary)]"
                  >
                    {t('delete.title')}
                  </h3>
                  <p className="mt-0.5 text-xs text-[var(--text-muted)]">{target.name}</p>
                </div>
              </div>

              <button
                type="button"
                onClick={onClose}
                disabled={isDeleting}
                aria-label={t('common:close')}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="space-y-3.5 px-5 py-5">
              {/* 待删除对象核对区 */}
              <div className="flex items-center gap-3 rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/40 p-3">
                <div className="relative h-12 w-12 shrink-0 overflow-hidden rounded-xl border border-[var(--border)]/80 bg-[var(--bg-secondary)]">
                  {avatarUrl ? (
                    <img src={avatarUrl} alt={target.name} className="h-full w-full object-cover" />
                  ) : (
                    <div className="flex h-full w-full items-center justify-center text-[var(--text-muted)]">
                      <User className="h-5 w-5 opacity-50" aria-hidden="true" />
                    </div>
                  )}
                </div>

                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-semibold text-[var(--text-primary)]">
                    {target.name}
                  </p>
                  <span className="font-data mt-1 inline-flex items-center gap-1 rounded-md border border-[var(--status-success-border)] bg-[var(--status-success-soft)] px-1.5 py-0.5 text-[11px] font-semibold text-[var(--status-success)] tabular-nums select-text">
                    <span className="text-[9px] font-medium text-[var(--status-success)]/70">
                      ID
                    </span>
                    <span>{target.subjectId}</span>
                  </span>
                </div>

                <span className="font-data shrink-0 rounded-lg border border-[var(--border)]/70 bg-[var(--bg-surface)]/70 px-2 py-1 text-[10px] font-semibold text-[var(--text-secondary)] tabular-nums">
                  {t('card.sampleCount')} {target.faceCount}/5
                </span>
              </div>

              {/* 影响范围与不可逆提示 */}
              <div className="flex items-start gap-2.5 rounded-2xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] p-3.5">
                <AlertTriangle
                  className="mt-0.5 h-4 w-4 shrink-0 text-[var(--status-danger)]"
                  aria-hidden="true"
                />
                <p className="text-xs leading-relaxed text-[var(--text-secondary)]">
                  {t('delete.desc', {
                    name: target.name,
                    subjectId: target.subjectId,
                  })}
                </p>
              </div>
            </div>

            {/* 吸底操作栏 */}
            <div className="flex items-center justify-between gap-3 border-t border-[var(--border)]/70 px-5 py-4">
              <div className="hidden items-center gap-1 text-[11px] text-[var(--text-muted)] sm:flex">
                <span>{t('modal.escHintPrefix')}</span>
                <kbd className="rounded border border-[var(--border)] bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)] shadow-xs">
                  ESC
                </kbd>
                <span>{t('modal.escHintSuffix')}</span>
              </div>

              <div className="flex items-center gap-2.5">
                <button
                  type="button"
                  onClick={onClose}
                  disabled={isDeleting}
                  className="inline-flex h-9 items-center justify-center rounded-xl border border-[var(--border)] px-4 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
                >
                  {t('actions.cancel')}
                </button>
                <button
                  type="button"
                  onClick={onConfirm}
                  disabled={isDeleting}
                  className="inline-flex h-9 items-center gap-2 rounded-xl bg-[var(--status-danger-solid)] px-4 text-xs font-semibold text-white shadow-xs transition-colors hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--status-danger)]/50 focus-visible:outline-none disabled:opacity-50"
                >
                  {isDeleting && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                  {isDeleting ? t('actions.delete') : t('actions.confirm')}
                </button>
              </div>
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}
