import React from 'react'
import { Check, CheckCircle2 } from 'lucide-react'

export interface AlarmStatusButtonProps {
  isProcessed: boolean
  onClick: (e: React.MouseEvent) => void
  t: (key: string) => string
  className?: string
}

export function AlarmStatusButton({
  isProcessed,
  onClick,
  t,
  className = '',
}: AlarmStatusButtonProps): React.ReactElement {
  if (isProcessed) {
    return (
      <button
        type="button"
        onClick={onClick}
        className={`flex items-center gap-1.5 rounded-lg border border-[var(--status-success-border)] bg-[var(--status-success-soft)] px-3 py-1 text-xs font-semibold text-[var(--status-success)] transition-all hover:bg-[var(--status-success-soft)] ${className}`}
        title={t('card.processed')}
      >
        <CheckCircle2 className="h-3.5 w-3.5" />
        <span>{t('card.processed')}</span>
      </button>
    )
  }

  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex items-center gap-1.5 rounded-lg border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] px-3 py-1 text-xs font-semibold text-[var(--status-danger)] transition-all hover:bg-[var(--status-danger-solid)] hover:text-white ${className}`}
      title={t('card.markProcessed')}
    >
      <Check className="h-3.5 w-3.5" />
      <span>{t('card.markProcessed')}</span>
    </button>
  )
}
