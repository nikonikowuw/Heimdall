import React, { useCallback, useEffect, useState } from 'react'
import { AlertCircle, AlertTriangle, Loader2, Trash2, X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { cameraApi } from '@/lib/api'
import type { Camera } from '@/types'

export interface DeleteCameraModalProps {
  isOpen: boolean
  camera: Camera | null
  onClose: () => void
  onSuccess: (cameraId: string) => void
}

export function DeleteCameraModal({
  isOpen,
  camera,
  onClose,
  onSuccess,
}: DeleteCameraModalProps): React.ReactElement | null {
  const { t } = useTranslation('camera')
  const { t: tc } = useTranslation('common')

  const [isDeleting, setIsDeleting] = useState(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)
  const [activeCamera, setActiveCamera] = useState<Camera | null>(camera)

  useEffect(() => {
    if (camera) setActiveCamera(camera)
  }, [camera])

  const currentCamera = camera ?? activeCamera

  const handleDelete = useCallback(async () => {
    if (!camera) return
    setIsDeleting(true)
    setErrorMsg(null)
    try {
      await cameraApi.delete(camera.cameraId)
      onSuccess(camera.cameraId)
      onClose()
    } catch (err) {
      const msg =
        err instanceof Error
          ? err.message
          : t('manage.deleteFailed', { defaultValue: '删除摄像头失败，请稍后重试' })
      setErrorMsg(msg)
    } finally {
      setIsDeleting(false)
    }
  }, [camera, onSuccess, onClose, t])

  // ESC 浮层栈支持；Enter 经同一栈分发，仅栈顶弹窗可接管确认
  useDismissStack(isOpen, onClose, { disabled: isDeleting, onConfirm: handleDelete })

  return (
    <AnimatePresence
      onExitComplete={() => {
        if (!camera) setActiveCamera(null)
      }}
    >
      {isOpen && currentCamera && (
        <div
          onClick={(e) => {
            if (e.target === e.currentTarget && !isDeleting) {
              onClose()
            }
          }}
          className="modal-backdrop modal-backdrop--top"
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 10 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 10 }}
            transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
            className="modal-surface modal-surface--compact"
          >
            {/* 危险警告头部 */}
            <div className="flex shrink-0 items-center justify-between border-b border-[var(--border)]/70 px-6 py-4.5">
              <div className="flex items-center gap-3.5">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)] shadow-xs">
                  <Trash2 className="h-5 w-5" />
                </div>
                <div>
                  <h3 className="text-base font-bold tracking-tight text-[var(--text-primary)]">
                    {t('manage.deleteTitle')}
                  </h3>
                  <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                    {t('manage.deleteMessage', {
                      name: currentCamera.name,
                      cameraId: currentCamera.cameraId,
                    })}
                  </p>
                </div>
              </div>
              <button
                type="button"
                onClick={onClose}
                disabled={isDeleting}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* 内容警告区 */}
            <div className="space-y-3.5 p-6">
              <div className="flex items-start gap-2.5 rounded-2xl border border-amber-500/30 bg-amber-500/10 p-3.5 text-xs text-amber-600 dark:text-amber-400">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                <div className="leading-relaxed">{t('manage.deleteWarning')}</div>
              </div>

              {/* 错误提示条 */}
              {errorMsg && (
                <div className="flex items-center gap-2 rounded-2xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] p-3 text-xs text-[var(--status-danger)]">
                  <AlertCircle className="h-4 w-4 shrink-0" />
                  <span>{errorMsg}</span>
                </div>
              )}
            </div>

            {/* 现代化底部按钮栏 */}
            <div className="flex shrink-0 items-center justify-between border-t border-[var(--border)]/70 bg-[var(--bg-secondary)]/20 px-6 py-4">
              <div className="hidden items-center gap-1 text-[11px] text-[var(--text-muted)] sm:flex">
                <span>{t('manage.pressKeyPrefix', { defaultValue: '按' })}</span>
                <kbd className="rounded border border-[var(--border)] bg-white px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)] shadow-xs dark:bg-[var(--bg-surface-solid)]">
                  ↵
                </kbd>
                <span>{t('manage.toConfirmDelete', { defaultValue: '确认删除' })}</span>
              </div>

              <div className="flex items-center gap-2.5">
                <button
                  type="button"
                  onClick={onClose}
                  disabled={isDeleting}
                  className="rounded-xl border border-[var(--border)]/80 bg-white px-4 py-2 text-xs font-medium text-[var(--text-secondary)] shadow-xs transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] disabled:opacity-50 dark:bg-[var(--bg-surface-solid)]"
                >
                  {t('manage.cancel', tc('actions.cancel'))}
                </button>
                <button
                  type="button"
                  onClick={handleDelete}
                  disabled={isDeleting}
                  className="flex min-h-9 items-center gap-2 rounded-xl bg-[var(--status-danger-solid)] px-5 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95 disabled:opacity-50"
                >
                  {isDeleting ? (
                    <>
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      <span>{t('manage.deleting')}</span>
                    </>
                  ) : (
                    <>
                      <Trash2 className="h-3.5 w-3.5" />
                      <span>{t('manage.confirmDelete')}</span>
                    </>
                  )}
                </button>
              </div>
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}
