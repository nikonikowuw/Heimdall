import type { MountInfo } from '@/types/system'
import { useTranslation } from 'react-i18next'

interface MountBadgeProps {
  mount?: MountInfo
  className?: string
}

export function MountBadge({ mount, className = '' }: MountBadgeProps): React.ReactElement | null {
  const { t } = useTranslation('system')

  if (!mount || !mount.mountPoint) {
    return null
  }

  return (
    <div className={`flex flex-wrap items-center gap-1.5 text-[11px] ${className}`}>
      <span className="inline-flex items-center gap-1 rounded border border-[var(--border)] bg-[var(--bg-secondary)]/50 px-1.5 py-0.5 font-mono text-[var(--text-secondary)]">
        <span className="text-[var(--text-muted)]">
          {t('mount.mountPoint', { defaultValue: '挂载点' })}:
        </span>
        {mount.mountPoint}
      </span>
      {mount.device && (
        <span className="inline-flex items-center gap-1 rounded border border-[var(--border)] bg-[var(--bg-secondary)]/50 px-1.5 py-0.5 font-mono text-[var(--text-muted)]">
          {mount.device}
        </span>
      )}
      {mount.fsType && (
        <span className="inline-flex items-center gap-1 rounded border border-[var(--border)] bg-[var(--bg-secondary)]/50 px-1.5 py-0.5 font-mono text-[var(--text-muted)]">
          {mount.fsType}
        </span>
      )}
    </div>
  )
}
