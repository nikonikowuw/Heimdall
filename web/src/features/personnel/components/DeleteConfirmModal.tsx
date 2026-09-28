import React from 'react'
import { Trash2, User } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import { evidenceApi } from '@/lib/api'
import { MAX_PERSONNEL_PHOTOS_PER_PERSON } from '@/lib/personnelUpload'
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
 *
 * 外壳（遮罩、层级、焦点陷阱、ESC/Enter）统一由共享 ConfirmDialog 承担。
 */
export function DeleteConfirmModal({
  isOpen,
  target,
  isDeleting,
  onClose,
  onConfirm,
}: DeleteConfirmModalProps): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])

  const avatarUrl = target?.primaryPhotoPath ? evidenceApi.getImageUrl(target.primaryPhotoPath) : ''

  return (
    <ConfirmDialog
      isOpen={isOpen && Boolean(target)}
      title={t('delete.title')}
      description={target?.name}
      confirmLabel={t('actions.confirm')}
      cancelLabel={t('common:cancel')}
      icon={Trash2}
      isConfirming={isDeleting}
      onConfirm={onConfirm}
      onClose={onClose}
      showKeyboardHint
    >
      {target && (
        <>
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
              <span className="font-data border-status-success-border bg-status-success-soft text-status-success mt-1 inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-[11px] font-semibold tabular-nums select-text">
                <span className="text-status-success/70 text-[9px] font-medium">ID</span>
                <span>{target.subjectId}</span>
              </span>
            </div>

            <span className="font-data shrink-0 rounded-lg border border-[var(--border)]/70 bg-[var(--bg-surface)]/70 px-2 py-1 text-[10px] font-semibold text-[var(--text-secondary)] tabular-nums">
              {t('card.sampleCount')} {target.faceCount}/{MAX_PERSONNEL_PHOTOS_PER_PERSON}
            </span>
          </div>

          {/* 影响范围与不可逆提示 */}
          <div className="border-status-danger-border bg-status-danger-soft flex items-start gap-2.5 rounded-2xl border p-3.5">
            <Trash2 className="text-status-danger mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
            <p className="text-xs leading-relaxed text-[var(--text-secondary)]">
              {t('delete.desc', {
                name: target.name,
                subjectId: target.subjectId,
              })}
            </p>
          </div>
        </>
      )}
    </ConfirmDialog>
  )
}
