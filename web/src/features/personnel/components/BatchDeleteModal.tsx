import React from 'react'
import { Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import type { PersonnelItem } from '@/types'

export interface BatchDeleteModalProps {
  isOpen: boolean
  targets: PersonnelItem[]
  isDeleting: boolean
  currentProgress?: { current: number; total: number } | null
  onClose: () => void
  onConfirm: () => void
}

/**
 * 人员批量删除二次确认。
 *
 * 外壳、层级与焦点约束由共享 ConfirmDialog 承担；本组件提供待删除清单预览与
 * 批量进度反馈——批量操作不可逆且影响面大于单人，清单必须可见可核对。
 */
export function BatchDeleteModal({
  isOpen,
  targets,
  isDeleting,
  currentProgress,
  onClose,
  onConfirm,
}: BatchDeleteModalProps): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])

  return (
    <ConfirmDialog
      isOpen={isOpen}
      title={t('batch.deleteConfirmTitle')}
      description={t('batch.deleteConfirmDesc', { count: targets.length })}
      confirmLabel={isDeleting ? t('actions.saving') : t('batch.batchDelete')}
      cancelLabel={t('common:cancel')}
      icon={Trash2}
      isConfirming={isDeleting}
      onConfirm={onConfirm}
      onClose={onClose}
      priority={10}
    >
      {/* 待删除人员预览清单 */}
      <div className="max-h-36 overflow-y-auto rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/50 p-2 text-xs">
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

      {isDeleting && currentProgress && (
        <p className="text-status-danger text-xs" role="status">
          {t('batch.deleting', {
            current: currentProgress.current,
            total: currentProgress.total,
          })}
        </p>
      )}
    </ConfirmDialog>
  )
}
