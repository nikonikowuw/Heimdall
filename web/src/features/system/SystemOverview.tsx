/**
 * 系统概述模块
 *
 * 工业级硬件监控仪表盘，显示系统资源状态、NPU 多核心状态、
 * 网络流量、温度和存储使用情况。
 */

import { useState, useEffect, useCallback, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import type { LucideIcon } from 'lucide-react'
import { Server, Cpu, HardDrive, Camera, AlertTriangle, Activity, Clock } from 'lucide-react'
import { systemApi } from '../../lib/system-api'
import { RefreshButton } from '../../components/RefreshButton'
import { SettingsSection, LoadingSkeleton, ErrorBanner } from './components/SettingsSection'
import type { SystemOverview as SystemOverviewData } from '../../types/system'

// 导入新的组件
import { CpuHeatmap } from './components/CpuHeatmap'
import { NpuOverview } from './components/NpuOverview'
import { NetworkChart } from './components/NetworkChart'
import { ThermalStatus } from './components/ThermalStatus'
import { MemoryDetail } from './components/MemoryDetail'
import { DiskDetail } from './components/DiskDetail'
import { ProcessList } from './components/ProcessList'

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
  const [refreshing, setRefreshing] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const dataRef = useRef<SystemOverviewData | null>(null)
  dataRef.current = data

  const loadData = useCallback(
    async (isManual = false, signal?: AbortSignal) => {
      try {
        if (isManual) {
          setRefreshing(true)
        }
        const overview = await systemApi.getOverview(signal)
        if (!signal?.aborted) {
          setData(overview)
          setError(null)
        }
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return
        if (!signal?.aborted) {
          // 仅在初始未载入或手动刷新失败时抛出错误横幅，避免后台静默采集闪烁打断
          if (!dataRef.current || isManual) {
            setError(
              err instanceof Error
                ? err.message
                : t('loadFailed', { defaultValue: 'Failed to load' }),
            )
          }
        }
      } finally {
        if (!signal?.aborted) {
          setLoading(false)
          setRefreshing(false)
        }
      }
    },
    [t],
  )

  // 定时 2s 轮询采集，并在页面隐藏时挂起节能（防重叠与防 AbortController 泄露）
  useEffect(() => {
    let activeController: AbortController | null = null
    let timerId: ReturnType<typeof setTimeout> | null = null
    let isMounted = true

    const scheduleNext = (delayMs: number) => {
      if (!isMounted) return
      if (timerId) clearTimeout(timerId)
      timerId = setTimeout(executePoll, delayMs)
    }

    const executePoll = async () => {
      if (!isMounted) return
      if (document.visibilityState !== 'visible') {
        scheduleNext(2000)
        return
      }

      // 中断仍在飞行的上一次请求，防止网络波动堆积
      if (activeController) {
        activeController.abort()
      }
      const controller = new AbortController()
      activeController = controller

      try {
        await loadData(false, controller.signal)
      } finally {
        if (isMounted) {
          if (activeController === controller) {
            activeController = null
          }
          scheduleNext(2000)
        }
      }
    }

    // 首次进入立即触发加载
    executePoll()

    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible' && isMounted) {
        executePoll()
      }
    }
    document.addEventListener('visibilitychange', handleVisibilityChange)

    return () => {
      isMounted = false
      if (timerId) clearTimeout(timerId)
      document.removeEventListener('visibilitychange', handleVisibilityChange)
      if (activeController) {
        activeController.abort()
        activeController = null
      }
    }
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
            {t('overview.subtitle', {
              defaultValue: '设备运行状态与资源概览',
            })}
          </p>
        </div>
        <div className="flex items-center gap-2.5">
          <div className="flex items-center gap-1.5 rounded-full border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1 text-[11px] text-[var(--text-muted)]">
            <span className="relative flex h-2 w-2">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-[var(--accent-green)] opacity-75" />
              <span className="relative inline-flex h-2 w-2 rounded-full bg-[var(--accent-green)]" />
            </span>
            <span>{t('overview.live', { defaultValue: 'Live (2s)' })}</span>
          </div>
          <RefreshButton onClick={() => loadData(true)} loading={refreshing} />
        </div>
      </div>

      {error && <ErrorBanner message={error} onRetry={() => loadData(true)} />}

      {/* 首次加载时所有区块显示骨架屏 */}
      {loading && !data && (
        <>
          <SettingsSection title={t('overview.resources', { defaultValue: '资源状态' })}>
            <LoadingSkeleton rows={2} />
          </SettingsSection>
          <SettingsSection title={t('overview.cpuCores', { defaultValue: 'CPU 核心' })}>
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
              {Array.from({ length: 4 }).map((_, i) => (
                <div key={i} className="h-28 animate-pulse rounded-xl bg-[var(--bg-secondary)]" />
              ))}
            </div>
          </SettingsSection>
          <SettingsSection title={t('overview.memoryDetail', { defaultValue: '内存详情' })}>
            <LoadingSkeleton rows={3} />
          </SettingsSection>
          <SettingsSection title={t('overview.network', { defaultValue: '网络流量' })}>
            <LoadingSkeleton rows={2} />
          </SettingsSection>
          <SettingsSection title={t('overview.diskDetail', { defaultValue: '磁盘详情' })}>
            <LoadingSkeleton rows={2} />
          </SettingsSection>
        </>
      )}

      {/* 数据就绪后显示实际内容 */}
      {data && (
        <SettingsSection title={t('overview.resources', { defaultValue: '资源状态' })}>
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
        </SettingsSection>
      )}

      {/* CPU 核心热力图 - 新增 */}
      {data && data.cpu.perCore.length > 0 && (
        <SettingsSection title={t('overview.cpuCores', { defaultValue: 'CPU 核心' })}>
          <CpuHeatmap cores={data.cpu.perCore} />
        </SettingsSection>
      )}

      {/* Top 5 进程资源消耗 - 新增 */}
      {data && data.cpu.topProcesses && data.cpu.topProcesses.length > 0 && (
        <SettingsSection title={t('overview.topProcesses', { defaultValue: '进程资源消耗排行' })}>
          <ProcessList processes={data.cpu.topProcesses} />
        </SettingsSection>
      )}

      {/* NPU 多核心概览 - 新增 */}
      {data && data.npu && (
        <SettingsSection title={t('overview.npu', { defaultValue: 'NPU 状态' })}>
          <NpuOverview npu={data.npu} />
        </SettingsSection>
      )}

      {/* 内存详细统计 - 新增 */}
      {data && (
        <SettingsSection title={t('overview.memoryDetail', { defaultValue: '内存详情' })}>
          <MemoryDetail memory={data.memory} />
        </SettingsSection>
      )}

      {/* 网络流量 - 新增 */}
      {data && data.network.length > 0 && (
        <SettingsSection title={t('overview.network', { defaultValue: '网络流量' })}>
          <NetworkChart interfaces={data.network} />
        </SettingsSection>
      )}

      {/* 温度状态 - 新增 */}
      {data && data.thermal.zones.length > 0 && (
        <SettingsSection title={t('overview.thermal', { defaultValue: '温度状态' })}>
          <ThermalStatus thermal={data.thermal} />
        </SettingsSection>
      )}

      {/* 磁盘详细统计 - 新增 */}
      {data && (
        <SettingsSection title={t('overview.diskDetail', { defaultValue: '磁盘详情' })}>
          <DiskDetail disk={data.disk} />
        </SettingsSection>
      )}

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
