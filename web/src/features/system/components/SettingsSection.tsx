import { useTranslation } from 'react-i18next'

interface SettingsSectionProps {
  title: string
  description?: string
  action?: React.ReactNode
  children: React.ReactNode
  className?: string
}

export function SettingsSection({
  title,
  description,
  action,
  children,
  className = '',
}: SettingsSectionProps): React.ReactElement {
  return (
    <section className={`settings-card ${className}`}>
      <div className="mb-5 flex items-start justify-between">
        <div>
          <h3 className="text-[15px] font-semibold text-[var(--text-primary)]">{title}</h3>
          {description && (
            <p className="mt-1 text-[13px] leading-relaxed text-[var(--text-muted)]">
              {description}
            </p>
          )}
        </div>
        {action}
      </div>
      {children}
    </section>
  )
}

interface LoadingSkeletonProps {
  rows?: number
  className?: string
}

const SKELETON_WIDTHS = ['72%', '85%', '63%', '91%', '78%', '68%', '88%', '74%']

export function LoadingSkeleton({
  rows = 3,
  className = '',
}: LoadingSkeletonProps): React.ReactElement {
  return (
    <div className={`space-y-3 ${className}`}>
      {Array.from({ length: rows }).map((_, i) => (
        <div key={i} className="flex items-center gap-4">
          <div className="h-4 w-24 animate-pulse rounded-md bg-[var(--bg-secondary)]" />
          <div
            className="h-4 flex-1 animate-pulse rounded-md bg-[var(--bg-secondary)]"
            style={{ maxWidth: SKELETON_WIDTHS[i % SKELETON_WIDTHS.length] }}
          />
        </div>
      ))}
    </div>
  )
}

interface ErrorBannerProps {
  message: string
  onRetry?: () => void
}

export function ErrorBanner({ message, onRetry }: ErrorBannerProps): React.ReactElement {
  const { t } = useTranslation('system')
  return (
    <div className="flex items-center gap-3 rounded-xl border border-[var(--destructive)]/20 bg-[var(--destructive)]/5 px-4 py-3">
      <div className="h-2 w-2 shrink-0 rounded-full bg-[var(--destructive)]" />
      <p className="flex-1 text-[13px] text-[var(--destructive)]">{message}</p>
      {onRetry && (
        <button
          onClick={onRetry}
          className="shrink-0 text-[13px] font-medium text-[var(--destructive)] underline underline-offset-2 hover:no-underline"
        >
          {t('retry', { defaultValue: 'Retry' })}
        </button>
      )}
    </div>
  )
}
