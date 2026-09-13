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
        className={`flex items-center gap-1.5 rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-3 py-1 text-xs font-semibold text-emerald-500 transition-all hover:bg-emerald-500/25 ${className}`}
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
      className={`flex items-center gap-1.5 rounded-lg border border-rose-500/30 bg-rose-500/15 px-3 py-1 text-xs font-semibold text-rose-500 transition-all hover:bg-rose-500 hover:text-white ${className}`}
      title={t('card.markProcessed')}
    >
      <Check className="h-3.5 w-3.5" />
      <span>{t('card.markProcessed')}</span>
    </button>
  )
}
