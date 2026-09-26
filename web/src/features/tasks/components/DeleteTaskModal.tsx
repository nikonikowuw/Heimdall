import React, { useCallback, useState } from 'react'
import { AlertCircle, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import { taskApi } from '@/lib/api'

export interface DeleteTaskModalProps {
  isOpen: boolean
  cameraId: string | null
  taskName: string | null
  onClose: () => void
  onSuccess: (cameraId: string) => void
}

/**
 * 删除布防任务二次确认。
 *
 * 外壳（遮罩、层级、焦点陷阱、ESC/Enter）统一由共享 ConfirmDialog 承担；
 * 本组件只负责删除请求、错误呈现与「影响范围」说明。
 */
export function DeleteTaskModal({
  isOpen,
  cameraId,
  taskName,
  onClose,
  onSuccess,
}: DeleteTaskModalProps): React.ReactElement {
  const { t } = useTranslation(['task', 'common'])

  const [isDeleting, setIsDeleting] = useState(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)

  const handleDelete = useCallback(async () => {
    if (!cameraId) return
    setIsDeleting(true)
    setErrorMsg(null)
    try {
      await taskApi.deleteTask(cameraId)
      onSuccess(cameraId)
      onClose()
    } catch (err) {
      const msg = err instanceof Error ? err.message : t('errors.deleteFailed')
      setErrorMsg(msg)
    } finally {
      setIsDeleting(false)
    }
  }, [cameraId, onSuccess, onClose, t])

  const displayName = taskName || cameraId || ''

  return (
    <ConfirmDialog
      isOpen={isOpen && Boolean(cameraId)}
      title={t('deleteTaskTitle')}
      description={t('deleteTaskMessage', { name: displayName })}
      confirmLabel={isDeleting ? t('deletingTask') : t('confirmDeleteTask')}
      cancelLabel={t('common:cancel')}
      icon={Trash2}
      isConfirming={isDeleting}
      errorMessage={errorMsg}
      onConfirm={handleDelete}
      onClose={onClose}
      showKeyboardHint
    >
      {taskName && cameraId && (
        <div className="flex items-center justify-between gap-3 rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] px-3 py-2 text-xs">
          <span className="text-[var(--text-secondary)]">{t('deleteTaskCameraId')}</span>
          <code className="min-w-0 truncate font-mono text-[var(--text-primary)]">{cameraId}</code>
        </div>
      )}
      <div className="border-status-warning-border bg-status-warning-soft text-status-warning flex items-start gap-2 rounded-xl border p-3 text-xs">
        <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        <span className="leading-relaxed">{t('deleteTaskWarning')}</span>
      </div>
    </ConfirmDialog>
  )
}
