import type { ReactElement } from 'react'
import { LayoutList, Rows3 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { RefreshButton } from '@/components/RefreshButton'

export type LogDensity = 'comfortable' | 'compact'

interface OplogToolbarProps {
  isLoading: boolean
  onRefresh: () => void
  density: LogDensity
  onDensityChange: (density: LogDensity) => void
  /** 当前页记录数；筛选在服务端完成，因此不存在单独的客户端命中数 */
  pageCount: number
}

export function OplogToolbar({
  isLoading,
  onRefresh,
  density,
  onDensityChange,
  pageCount,
}: OplogToolbarProps): ReactElement {
  const { t } = useTranslation('oplog')

  return (
    <div className="flex flex-wrap items-center gap-2">
      {/* 当前页计数指示 */}
      <span className="font-data hidden text-[11px] text-[var(--text-muted)] xl:inline-block">
        {t('toolbar.pageCount', { count: pageCount })}
      </span>

      {/* 视图密度切换按钮 (舒适 / 紧凑) */}
      <div className="flex items-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] p-0.5">
        <button
          type="button"
          onClick={() => onDensityChange('comfortable')}
          aria-label={t('toolbar.densityComfortable')}
          aria-pressed={density === 'comfortable'}
          title={t('toolbar.densityComfortable')}
          className={`flex h-7 w-7 items-center justify-center rounded-md transition-colors ${
            density === 'comfortable'
              ? 'bg-[var(--accent-soft)] text-[var(--accent)] shadow-2xs'
              : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
          }`}
        >
          <Rows3 className="h-3.5 w-3.5" />
        </button>
        <button
          type="button"
          onClick={() => onDensityChange('compact')}
          aria-label={t('toolbar.densityCompact')}
          aria-pressed={density === 'compact'}
          title={t('toolbar.densityCompact')}
          className={`flex h-7 w-7 items-center justify-center rounded-md transition-colors ${
            density === 'compact'
              ? 'bg-[var(--accent-soft)] text-[var(--accent)] shadow-2xs'
              : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
          }`}
        >
          <LayoutList className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* 手动刷新主按钮 */}
      <RefreshButton
        onClick={onRefresh}
        loading={isLoading}
        label={t('refreshStream')}
        className="reticle-target"
      />
    </div>
  )
}
