import type { ReactElement } from 'react'
import type { LucideIcon } from 'lucide-react'
import { CloseIconButton } from './CloseIconButton'

export interface ModalFormHeaderProps {
  icon: LucideIcon
  title: string
  titleId: string
  description: string
  descriptionId: string
  closeLabel: string
  closeTitle?: string
  onClose: () => void
  closeDisabled?: boolean
  badge?: string
}

export function ModalFormHeader({
  icon: Icon,
  title,
  titleId,
  description,
  descriptionId,
  closeLabel,
  closeTitle,
  onClose,
  closeDisabled = false,
  badge,
}: ModalFormHeaderProps): ReactElement {
  return (
    <header className="modal-form-header">
      <div className="flex min-w-0 items-center gap-3">
        <div className="modal-form-icon" aria-hidden="true">
          <Icon className="h-5 w-5" />
        </div>
        <div className="min-w-0">
          <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
            <h2
              id={titleId}
              className="text-base leading-6 font-semibold text-[var(--text-primary)]"
            >
              {title}
            </h2>
            {badge && <span className="modal-form-badge">{badge}</span>}
          </div>
          <p id={descriptionId} className="mt-0.5 text-xs leading-5 text-[var(--text-secondary)]">
            {description}
          </p>
        </div>
      </div>
      <CloseIconButton
        onClick={onClose}
        label={closeLabel}
        title={closeTitle}
        disabled={closeDisabled}
        // 关闭控件按样式规范使用 --text-secondary（CloseIconButton plain 默认是 --text-muted）
        className="text-[var(--text-secondary)]"
      />
    </header>
  )
}
