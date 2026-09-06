import { useState, useEffect, useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import type { LucideIcon } from 'lucide-react'
import { Server, Cpu, HardDrive, Camera, AlertTriangle, Activity, Clock } from 'lucide-react'
import { systemApi } from '../../lib/system-api'
import { RefreshButton } from '../../components/RefreshButton'
import { SettingsSection, LoadingSkeleton, ErrorBanner } from './components/SettingsSection'
import type { SystemOverview as SystemOverviewData } from '../../types/system'

function formatUptime(
  seconds: number,
  t: (key: string, options?: { defaultValue?: string }) => string,
): string {
  const days = Math.floor(seconds / 86400)
  const hours = Math.floor((seconds % 86400) / 3600)
  const mins = Math.floor((seconds % 3600) / 60)
  const dUnit = t('overview.days', { defaultValue: 'd' })
  const hUnit = t('overview.hours', { defaultValue: 'h' })
  const mUnit = t('overview.mins', { defaultValue: 'm' })
  if (days > 0) return `${days}${dUnit} ${hours}${hUnit} ${mins}${mUnit}`
  if (hours > 0) return `${hours}${hUnit} ${mins}${mUnit}`
  return `${mins}${mUnit}`
}

function RingGauge({
  value,
  max = 100,
  size = 64,
  stroke = 5,
  color = 'var(--accent)',
  bgColor = 'var(--bg-secondary)',
}: {
  value: number
  max?: number
  size?: number
  stroke?: number
  color?: string
  bgColor?: string
}): React.ReactElement {
  const radius = (size - stroke) / 2
  const circumference = 2 * Math.PI * radius
  const percent = Math.min(value / max, 1)
  const offset = circumference * (1 - percent)

  return (
    <svg width={size} height={size} className="shrink-0 -rotate-90">
      <circle
        cx={size / 2}
        cy={size / 2}
        r={radius}
        fill="none"
        stroke={bgColor}
        strokeWidth={stroke}
      />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={radius}
        fill="none"
        stroke={color}
        strokeWidth={stroke}
        strokeDasharray={circumference}
        strokeDashoffset={offset}
        strokeLinecap="round"
        className="transition-all duration-700 ease-out"
      />
    </svg>
  )
}

function getUsageColor(percent: number): string {
  if (percent >= 95) return 'var(--destructive)'
  if (percent >= 80) return 'var(--accent-amber)'
  return 'var(--accent-green)'
}

export function SystemOverview(): React.ReactElement {
  const { t } = useTranslation('system')
  const [data, setData] = useState<SystemOverviewData | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const loadData = useCallback(
    async (signal?: AbortSignal) => {
      try {
        setLoading(true)
        setError(null)
        const overview = await systemApi.getOverview(signal)
        if (!signal?.aborted) setData(overview)
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return
        if (!signal?.aborted)
          setError(
            err instanceof Error
              ? err.message
              : t('loadFailed', { defaultValue: 'Failed to load' }),
          )
      } finally {
        if (!signal?.aborted) setLoading(false)
      }
    },
    [t],
  )

  useEffect(() => {
    const controller = new AbortController()
    loadData(controller.signal)
    return () => controller.abort()
  }, [loadData])

  return (
    <div className="space-y-5">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold text-[var(--text-primary)]">
            {t('overview.title', { defaultValue: '系统概览' })}
          </h2>
          <p className="mt-0.5 text-[13px] text-[var(--text-muted)]">
            {t('overview.subtitle', { defaultValue: '设备运行状态与资源概览' })}
          </p>
        </div>
        <RefreshButton onClick={() => loadData()} loading={loading} />
      </div>

      {error && <ErrorBanner message={error} onRetry={() => loadData()} />}

      {/* Resource gauges */}
      <SettingsSection title={t('overview.resources', { defaultValue: '资源状态' })}>
        {loading && !data ? (
          <LoadingSkeleton rows={4} />
        ) : data ? (
          <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
            <GaugeCard icon={Cpu} label="CPU" value={data.cpuUsagePercent} />
            <GaugeCard
              icon={HardDrive}
              label={t('overview.memory', { defaultValue: '内存' })}
              value={data.memoryUsagePercent}
              detail={`${data.memoryUsedMb}MB / ${data.memoryTotalMb}MB`}
            />
            {data.npuUsagePercent !== null && (
              <GaugeCard icon={Cpu} label="NPU" value={data.npuUsagePercent} />
            )}
            <GaugeCard
              icon={HardDrive}
              label={t('overview.disk', { defaultValue: '磁盘' })}
              value={data.diskUsagePercent}
              detail={`${data.diskUsedGb}GB / ${data.diskTotalGb}GB`}
            />
          </div>
        ) : null}
      </SettingsSection>

      {/* Device info + Business status side by side */}
      <div className="grid gap-5 lg:grid-cols-2">
        <SettingsSection title={t('overview.deviceInfo', { defaultValue: '设备信息' })}>
          {loading && !data ? (
            <LoadingSkeleton rows={5} />
          ) : data ? (
            <div className="space-y-2.5">
              <InfoRow
                icon={Server}
                label={t('overview.version', { defaultValue: '软件版本' })}
                value={data.softwareVersion}
              />
              <InfoRow
                icon={Server}
                label={t('overview.model', { defaultValue: '设备型号' })}
                value={data.deviceModel}
              />
              <InfoRow
                icon={Server}
                label={t('overview.os', { defaultValue: '操作系统' })}
                value={data.osInfo}
              />
              <InfoRow
                icon={Server}
                label={t('overview.kernel', { defaultValue: '内核版本' })}
                value={data.kernelVersion}
              />
              <InfoRow
                icon={Clock}
                label={t('overview.uptime', { defaultValue: '运行时间' })}
                value={formatUptime(data.uptimeSeconds, t)}
              />
            </div>
          ) : null}
        </SettingsSection>

        <SettingsSection title={t('overview.business', { defaultValue: '业务状态' })}>
          {loading && !data ? (
            <LoadingSkeleton rows={4} />
          ) : data ? (
            <div className="grid grid-cols-2 gap-3">
              <StatTile
                icon={Camera}
                label={t('overview.activeCameras', { defaultValue: '活跃摄像头' })}
                value={data.activeCameras}
                suffix={` / ${data.totalCameras}`}
                color="var(--accent)"
              />
              <StatTile
                icon={Activity}
                label={t('overview.activeTasks', { defaultValue: '活跃任务' })}
                value={data.activeTasks}
                color="var(--accent-green)"
              />
              <StatTile
                icon={AlertTriangle}
                label={t('overview.todayAlarms', { defaultValue: '今日告警' })}
                value={data.todayAlarms}
                color="var(--accent-amber)"
              />
              <StatTile
                icon={Camera}
                label={t('overview.todayCaptures', { defaultValue: '今日抓拍' })}
                value={data.todayCaptures}
                color="var(--accent)"
              />
            </div>
          ) : null}
        </SettingsSection>
      </div>
    </div>
  )
}

function GaugeCard({
  icon: Icon,
  label,
  value,
  detail,
}: {
  icon: LucideIcon
  label: string
  value: number
  detail?: string
}): React.ReactElement {
  const color = getUsageColor(value)
  return (
    <div className="gauge-card">
      <div className="relative">
        <RingGauge value={value} color={color} />
        <div className="absolute inset-0 flex items-center justify-center">
          <span className="text-xs font-bold tabular-nums" style={{ color }}>
            {Math.round(value)}
          </span>
        </div>
      </div>
      <div className="text-center">
        <div className="flex items-center justify-center gap-1.5">
          <Icon className="h-3 w-3 text-[var(--text-muted)]" />
          <span className="text-[13px] font-medium text-[var(--text-secondary)]">{label}</span>
        </div>
        {detail && (
          <p className="mt-0.5 text-[11px] text-[var(--text-muted)] tabular-nums">{detail}</p>
        )}
      </div>
    </div>
  )
}

function InfoRow({
  icon: Icon,
  label,
  value,
}: {
  icon: LucideIcon
  label: string
  value: string
}): React.ReactElement {
  return (
    <div className="info-row">
      <div className="flex items-center gap-2.5">
        <Icon className="h-3.5 w-3.5 text-[var(--text-muted)]" />
        <span className="text-[13px] text-[var(--text-secondary)]">{label}</span>
      </div>
      <span className="font-mono text-[13px] font-medium text-[var(--text-primary)]">{value}</span>
    </div>
  )
}

function StatTile({
  icon: Icon,
  label,
  value,
  suffix,
  color,
}: {
  icon: LucideIcon
  label: string
  value: number
  suffix?: string
  color: string
}): React.ReactElement {
  return (
    <div className="flex items-center gap-3 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5 transition-all hover:border-[var(--border-strong)] hover:shadow-sm">
      <div
        className="flex h-10 w-10 items-center justify-center rounded-xl"
        style={{ backgroundColor: `color-mix(in srgb, ${color} 10%, transparent)` }}
      >
        <Icon className="h-5 w-5" style={{ color }} />
      </div>
      <div>
        <p className="text-[11px] text-[var(--text-muted)]">{label}</p>
        <p className="text-lg font-bold text-[var(--text-primary)] tabular-nums">
          {value}
          {suffix && <span className="text-sm font-normal text-[var(--text-muted)]">{suffix}</span>}
        </p>
      </div>
    </div>
  )
}
