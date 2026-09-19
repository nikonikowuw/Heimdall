import { useMemo, type ReactElement } from 'react'
import { AlertCircle, AlertTriangle, Database, Info } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { OperationalLog } from '@/types'
import type { LevelFilter } from '../logFilters'
import { summarizeOperationalLogs } from '../logMetrics'
import { LogStatCard } from './LogStatCard'

interface OperationalStatsCardsProps {
  logs: OperationalLog[]
  levelFilter: LevelFilter
  onSelectLevelFilter: (filter: LevelFilter) => void
}

/**
 * 运维事件指标卡。数值口径为当前页，每张卡片同时是服务端 level 筛选入口。
 */
export function OperationalStatsCards({
  logs,
  levelFilter,
  onSelectLevelFilter,
}: OperationalStatsCardsProps): ReactElement {
  const { t } = useTranslation('oplog')
  const metrics = useMemo(() => summarizeOperationalLogs(logs), [logs])

  const toggle = (filter: LevelFilter) => () => {
    onSelectLevelFilter(levelFilter === filter ? 'all' : filter)
  }

  return (
    <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-2 lg:grid-cols-4">
      <LogStatCard
        index={0}
        label={t('stats.pageEvents')}
        value={metrics.pageSize}
        icon={Database}
        tone="accent"
        detail={t('stats.pageEventsDesc')}
        onFilterToggle={() => onSelectLevelFilter('all')}
        isFilterActive={levelFilter === 'all'}
      />

      <LogStatCard
        index={1}
        label={t('stats.pageErrors')}
        value={metrics.errorCount}
        icon={AlertCircle}
        tone={metrics.errorCount > 0 ? 'danger' : 'neutral'}
        valueTone={metrics.errorCount > 0 ? 'danger' : undefined}
        detail={t('stats.pageErrorsDesc')}
        onFilterToggle={toggle('error')}
        isFilterActive={levelFilter === 'error'}
      />

      <LogStatCard
        index={2}
        label={t('stats.pageWarnings')}
        value={metrics.warnCount}
        icon={AlertTriangle}
        tone={metrics.warnCount > 0 ? 'warning' : 'neutral'}
        valueTone={metrics.warnCount > 0 ? 'warning' : undefined}
        detail={t('stats.pageWarningsDesc')}
        onFilterToggle={toggle('warn')}
        isFilterActive={levelFilter === 'warn'}
      />

      <LogStatCard
        index={3}
        label={t('stats.pageInfoEvents')}
        value={metrics.infoCount}
        icon={Info}
        tone="accent"
        detail={t('stats.pageInfoEventsDesc')}
        onFilterToggle={toggle('info')}
        isFilterActive={levelFilter === 'info'}
      />
    </div>
  )
}
