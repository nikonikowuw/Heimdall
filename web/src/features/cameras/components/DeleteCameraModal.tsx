import React, { useEffect, useState } from 'react'
import { AlertCircle, AlertTriangle, Loader2, Trash2, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
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

  // ESC 快捷键关闭
  useEffect(() => {
    if (!isOpen) return
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !isDeleting) {
        onClose()
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [isOpen, isDeleting, onClose])

  if (!isOpen || !camera) return null

  const handleDelete = async () => {
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
  }

  return (
    <div
      onClick={(e) => {
        if (e.target === e.currentTarget && !isDeleting) {
          onClose()
        }
      }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-xs"
    >
      <div className="frosted-glass relative w-full max-w-md rounded-2xl border border-[var(--border)] p-6 shadow-2xl transition-all">
        {/* 右上角关闭 */}
        <button
          type="button"
          onClick={onClose}
          disabled={isDeleting}
          className="absolute top-5 right-5 rounded-lg p-1 text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
        >
          <X className="h-4 w-4" />
        </button>

        {/* 危险警告头部 */}
        <div className="flex items-start gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-rose-500/30 bg-rose-500/15 text-rose-500">
            <Trash2 className="h-5 w-5" />
          </div>
          <div>
            <h3 className="text-base font-semibold text-[var(--text-primary)]">
              {t('manage.deleteTitle')}
            </h3>
            <p className="mt-1 text-xs text-[var(--text-secondary)]">
              {t('manage.deleteMessage', {
                name: camera.name,
                cameraId: camera.cameraId,
              })}
            </p>
          </div>
        </div>

        {/* 警告框 */}
        <div className="mt-4 flex items-start gap-2.5 rounded-xl border border-amber-500/30 bg-amber-500/10 p-3 text-xs text-amber-500 dark:text-amber-400">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
          <div className="leading-relaxed">{t('manage.deleteWarning')}</div>
        </div>

        {/* 错误提示条 */}
        {errorMsg && (
          <div className="mt-3 flex items-center gap-2 rounded-xl border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-400">
            <AlertCircle className="h-4 w-4 shrink-0" />
            <span>{errorMsg}</span>
          </div>
        )}

        {/* 底部按钮栏 */}
        <div className="mt-6 flex justify-end gap-2.5">
          <button
            type="button"
            onClick={onClose}
            disabled={isDeleting}
            className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
          >
            {t('manage.cancel', tc('actions.cancel'))}
          </button>
          <button
            type="button"
            onClick={handleDelete}
            disabled={isDeleting}
            className="flex items-center gap-1.5 rounded-xl bg-rose-500 px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:bg-rose-600 active:scale-95 disabled:opacity-50"
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
    </div>
  )
}
