import { useMemo, type ReactElement } from 'react'
import { Activity, AlertTriangle, CheckCircle2, Clock } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { OperationLog } from '@/types'
import type { StatusFilter } from '../logFilters'
import { summarizeOperationLogs } from '../logMetrics'
import { latencyTone } from '../logTone'
import { LogStatCard } from './LogStatCard'

interface OperationStatsCardsProps {
  logs: OperationLog[]
  statusFilter: StatusFilter
  onSelectStatusFilter: (filter: StatusFilter) => void
}

/**
 * 操作审计指标卡。数值口径为当前页，异常请求卡同时是服务端 status=failed 的筛选入口。
 */
export function OperationStatsCards({
  logs,
  statusFilter,
  onSelectStatusFilter,
}: OperationStatsCardsProps): ReactElement {
  const { t } = useTranslation('oplog')
  const metrics = useMemo(() => summarizeOperationLogs(logs), [logs])

  const isErrorFilterActive = statusFilter === 'failed'

  return (
    <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-2 lg:grid-cols-4">
      <LogStatCard
        index={0}
        label={t('stats.pageRequests')}
        value={metrics.pageSize}
        icon={Activity}
        tone="accent"
        detail={t('stats.pageRequestsDesc')}
      />

      <LogStatCard
        index={1}
        label={t('stats.pageSuccessRate')}
        value={metrics.successRate}
        icon={CheckCircle2}
        tone="success"
        valueTone="success"
        detail={`${metrics.successCount} / ${metrics.pageSize}`}
      />

      <LogStatCard
        index={2}
        label={t('stats.pageErrors')}
        value={metrics.errorCount}
        icon={AlertTriangle}
        tone={metrics.errorCount > 0 ? 'danger' : 'neutral'}
        valueTone={metrics.errorCount > 0 ? 'danger' : undefined}
        detail={metrics.errorCount > 0 ? t('stats.viewErrorsOnly') : t('stats.pageErrorsDesc')}
        onFilterToggle={() => onSelectStatusFilter(isErrorFilterActive ? 'all' : 'failed')}
        isFilterActive={isErrorFilterActive}
      />

      <LogStatCard
        index={3}
        label={t('stats.pageAvgLatency')}
        value={t('durationValue', { value: metrics.avgDurationMs })}
        icon={Clock}
        tone={latencyTone(metrics.avgDurationMs)}
        detail={t(`stats.${metrics.latencyTier}`)}
      />
    </div>
  )
}
