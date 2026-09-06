import { RefreshCw } from 'lucide-react'
import { useTranslation } from 'react-i18next'

interface RefreshButtonProps {
  onClick: () => void
  loading?: boolean
}

export function RefreshButton({ onClick, loading = false }: RefreshButtonProps) {
  const { t } = useTranslation()

  return (
    <button
      onClick={onClick}
      disabled={loading}
      className="flex items-center gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-4 py-2 text-[13px] font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] active:scale-[0.97] disabled:opacity-50"
    >
      <RefreshCw className={`h-3.5 w-3.5 ${loading ? 'animate-spin' : ''}`} />
      {t('refresh', { defaultValue: '刷新' })}
    </button>
  )
}
