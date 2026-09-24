import React, { useCallback, useState } from 'react'
import { AlertCircle, AlertTriangle, Loader2, Trash2, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { taskApi } from '@/lib/api'

export interface DeleteTaskModalProps {
  isOpen: boolean
  cameraId: string | null
  taskName: string | null
  onClose: () => void
  onSuccess: (cameraId: string) => void
}

export function DeleteTaskModal({
  isOpen,
  cameraId,
  taskName,
  onClose,
  onSuccess,
}: DeleteTaskModalProps): React.ReactElement | null {
  const { t } = useTranslation('task')
  const { t: tc } = useTranslation('common')

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
      const msg =
        err instanceof Error
          ? err.message
          : t('errors.deleteFailed', { defaultValue: '删除任务失败，请稍后重试' })
      setErrorMsg(msg)
    } finally {
      setIsDeleting(false)
    }
  }, [cameraId, onSuccess, onClose, t])

  // ESC 浮层栈支持；Enter 经同一栈分发，仅栈顶弹窗可接管确认
  useDismissStack(isOpen && Boolean(cameraId), onClose, {
    disabled: isDeleting,
    onConfirm: handleDelete,
  })

  if (!isOpen || !cameraId) return null

  return (
    <div
      onClick={(e) => {
        if (e.target === e.currentTarget && !isDeleting) {
          onClose()
        }
      }}
      className="modal-backdrop"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="delete-task-title"
        className="modal-surface modal-surface--compact p-6"
      >
        {/* 右上角关闭 */}
        <button
          type="button"
          onClick={onClose}
          disabled={isDeleting}
          aria-label={tc('actions.cancel', { defaultValue: '取消' })}
          className="absolute top-5 right-5 rounded-[6px] p-1 text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
        >
          <X className="h-4 w-4" />
        </button>

        {/* 头部危险警示图标 */}
        <div className="flex items-center gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[8px] bg-[var(--status-warning-soft)] text-[var(--status-warning)]">
            <AlertTriangle className="h-5 w-5" />
          </div>
          <div>
            <h3 id="delete-task-title" className="text-base font-bold text-[var(--text-primary)]">
              {t('deleteTaskTitle', { defaultValue: '删除布防任务确认' })}
            </h3>
            <p className="font-mono text-xs text-[var(--text-muted)]">{cameraId}</p>
          </div>
        </div>

        {/* 提示文案 */}
        <div className="mt-4 space-y-2 text-xs leading-relaxed text-[var(--text-secondary)]">
          <p>
            {t('deleteTaskMessage', {
              name: taskName || cameraId,
              defaultValue: `确定要删除布防任务「${taskName || cameraId}」吗？`,
            })}
          </p>
          <div className="rounded-[8px] border border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] p-3 text-[var(--status-warning)]">
            <div className="flex items-start gap-2">
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span>
                {t('deleteTaskWarning', {
                  defaultValue:
                    '此操作仅从 AI 调度管线中移除该任务并释放推理算力。底层摄像头设备实体与历史告警将保留不受任何影响。',
                })}
              </span>
            </div>
          </div>
        </div>

        {/* 错误提示 */}
        {errorMsg && (
          <div className="mt-3 flex items-center gap-2 rounded-[8px] border border-[var(--status-danger)]/30 bg-[var(--status-danger)]/10 p-3 text-xs text-[var(--status-danger)]">
            <AlertCircle className="h-4 w-4 shrink-0" />
            <span>{errorMsg}</span>
          </div>
        )}

        {/* 底部按钮 */}
        <div className="mt-6 flex items-center justify-end gap-3">
          <button
            type="button"
            onClick={onClose}
            disabled={isDeleting}
            className="rounded-[6px] border border-[var(--border)] px-4 py-2 text-xs font-semibold text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
          >
            {tc('actions.cancel', { defaultValue: '取消' })}
          </button>
          <button
            type="button"
            onClick={handleDelete}
            disabled={isDeleting}
            className="flex items-center gap-1.5 rounded-[6px] bg-[var(--status-danger-solid)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
          >
            {isDeleting ? (
              <>
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                <span>{t('deletingTask', { defaultValue: '正在删除...' })}</span>
              </>
            ) : (
              <>
                <Trash2 className="h-3.5 w-3.5" />
                <span>{t('confirmDeleteTask', { defaultValue: '确认删除任务' })}</span>
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  )
}
