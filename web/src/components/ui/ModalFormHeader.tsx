import type { ReactElement } from 'react'
import { X, type LucideIcon } from 'lucide-react'

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
      <button
        type="button"
        onClick={onClose}
        disabled={closeDisabled}
        aria-label={closeLabel}
        title={closeTitle}
        className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
      >
        <X className="h-4 w-4" aria-hidden="true" />
      </button>
    </header>
  )
}
