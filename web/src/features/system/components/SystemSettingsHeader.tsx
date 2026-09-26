import type { ReactElement, ReactNode } from 'react'

export interface SystemSettingsHeaderProps {
  title: ReactNode
  description?: ReactNode
  actions?: ReactNode
}

export function SystemSettingsHeader({
  title,
  description,
  actions,
}: SystemSettingsHeaderProps): ReactElement {
  return (
    <header className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
      <div className="min-w-0 flex-1">
        <h2 className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-xl font-bold text-[var(--text-primary)]">
          {title}
        </h2>
        {description != null && (
          <p className="mt-0.5 text-[13px] text-[var(--text-muted)]">{description}</p>
        )}
      </div>
      {actions != null && (
        <div className="flex flex-wrap items-center justify-end gap-x-4 gap-y-2">{actions}</div>
      )}
    </header>
  )
}
