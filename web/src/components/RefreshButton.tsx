import type { ReactElement } from 'react'
import { RefreshCw } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'

export interface RefreshButtonProps {
  onClick: () => void
  loading?: boolean
  disabled?: boolean
  label?: string
  ariaLabel?: string
  title?: string
  className?: string
}

export function RefreshButton({
  onClick,
  loading = false,
  disabled = false,
  label,
  ariaLabel,
  title,
  className,
}: RefreshButtonProps): ReactElement {
  const { t } = useTranslation()
  const resolvedLabel = label ?? ariaLabel ?? title ?? t('refresh', { defaultValue: '刷新' })
  const resolvedTitle = title ?? label ?? ariaLabel ?? resolvedLabel
  const isButtonDisabled = disabled || loading

  return (
    <button
      type="button"
      onClick={onClick}
      disabled={isButtonDisabled}
      aria-label={resolvedLabel}
      title={resolvedTitle}
      className={cn('page-action-btn page-action-btn--icon', className)}
    >
      <RefreshCw className={cn('h-3.5 w-3.5', loading && 'animate-spin')} aria-hidden="true" />
    </button>
  )
}
