/**
 * NPU 多核心概览组件
 *
 * 显示 NPU 设备的多核心状态，包括使用率、频率和负载均衡度。
 * 支持 Rockchip RKNN（多核心）和 Ascend 设备。
 */

import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { Cpu } from 'lucide-react'
import type { NpuMetrics, NpuCoreMetrics } from '../../../types/system'
import { getUsageColor } from './colors'

interface NpuOverviewProps {
  npu: NpuMetrics
  className?: string
}

function calculateBalance(cores: NpuCoreMetrics[]): number {
  if (cores.length === 0) return 100

  const avg = cores.reduce((sum, c) => sum + c.utilizationPercent, 0) / cores.length
  if (avg === 0) return 100

  const variance =
    cores.reduce((sum, c) => {
      const diff = c.utilizationPercent - avg
      return sum + diff * diff
    }, 0) / cores.length

  // 标准差越小，均衡度越高
  const stdDev = Math.sqrt(variance)
  const balance = Math.max(0, 100 - stdDev)

  return Math.round(balance)
}

function getBalanceClass(balance: number): string {
  if (balance >= 80) return 'text-[var(--accent-green)]'
  if (balance >= 60) return 'text-[var(--accent-amber)]'
  return 'text-[var(--destructive)]'
}

function CoreBar({ core, maxUtilization }: { core: NpuCoreMetrics; maxUtilization: number }) {
  const color = getUsageColor(core.utilizationPercent)
  const heightPercent = maxUtilization > 0 ? (core.utilizationPercent / maxUtilization) * 100 : 0

  return (
    <div className="flex flex-col items-center gap-1">
      {/* 柱状图 */}
      <div className="relative h-16 w-8 overflow-hidden rounded-lg bg-[var(--bg-secondary)]">
        <div
          className="absolute right-0 bottom-0 left-0 rounded-lg transition-all duration-500 ease-out"
          style={{
            height: `${heightPercent}%`,
            backgroundColor: color,
          }}
        />
      </div>

      {/* 核心标签 */}
      <span className="text-[10px] font-medium text-[var(--text-muted)]">NPU{core.coreId}</span>

      {/* 使用率 */}
      <span className="text-xs font-bold tabular-nums" style={{ color }}>
        {Math.round(core.utilizationPercent)}
      </span>
    </div>
  )
}

function CoreCard({ core }: { core: NpuCoreMetrics }) {
  const { t } = useTranslation('system')
  const color = getUsageColor(core.utilizationPercent)

  return (
    <div
      className="flex flex-col items-center gap-2 rounded-xl border p-3 transition-all hover:scale-[1.02]"
      style={{
        borderColor: `color-mix(in srgb, ${color} 30%, transparent)`,
        backgroundColor: `color-mix(in srgb, ${color} 5%, transparent)`,
      }}
    >
      <span className="text-[11px] font-medium text-[var(--text-muted)]">
        {t('npu.core', { id: core.coreId, defaultValue: `Core ${core.coreId}` })}
      </span>

      {/* 使用率圆环 */}
      <div className="relative">
        <svg width={56} height={56} className="-rotate-90">
          <circle cx={28} cy={28} r={24} fill="none" stroke="var(--bg-secondary)" strokeWidth={4} />
          <circle
            cx={28}
            cy={28}
            r={24}
            fill="none"
            stroke={color}
            strokeWidth={4}
            strokeDasharray={150.8}
            strokeDashoffset={150.8 * (1 - core.utilizationPercent / 100)}
            strokeLinecap="round"
            className="transition-all duration-500 ease-out"
          />
        </svg>
        <div className="absolute inset-0 flex items-center justify-center">
          <span className="text-xs font-bold tabular-nums" style={{ color }}>
            {Math.round(core.utilizationPercent)}
          </span>
        </div>
      </div>

      {/* 频率 */}
      <span className="text-[10px] text-[var(--text-muted)] tabular-nums">
        {core.frequencyMhz} MHz
      </span>
    </div>
  )
}

export function NpuOverview({ npu, className = '' }: NpuOverviewProps) {
  const { t } = useTranslation('system')
  const balance = useMemo(() => calculateBalance(npu.cores), [npu.cores])
  const balanceClass = getBalanceClass(balance)

  // 计算平均使用率
  const avgUtilization = useMemo(() => {
    if (npu.cores.length === 0) return 0
    return npu.cores.reduce((sum, c) => sum + c.utilizationPercent, 0) / npu.cores.length
  }, [npu.cores])

  // 找到最大使用率（用于柱状图缩放）
  const maxUtilization = useMemo(() => {
    return Math.max(...npu.cores.map((c) => c.utilizationPercent), 1)
  }, [npu.cores])

  // 获取设备类型显示名
  const deviceTypeLabel = useMemo(() => {
    const dt = npu.deviceType.toLowerCase()
    if (dt.includes('rk3588')) {
      return t('npu.deviceTypes.rk3588', { defaultValue: 'Rockchip RK3588 NPU (3-Core)' })
    }
    if (dt.includes('rk3576')) {
      return t('npu.deviceTypes.rk3576', { defaultValue: 'Rockchip RK3576 NPU (2-Core)' })
    }
    if (dt.includes('rk3568')) {
      return t('npu.deviceTypes.rk3568', { defaultValue: 'Rockchip RK3568 NPU' })
    }
    if (dt.includes('rknn')) {
      return t('npu.deviceTypes.rknn', { defaultValue: 'Rockchip RKNN NPU' })
    }
    if (dt.includes('ascend')) {
      return t('npu.deviceTypes.ascend', { defaultValue: 'Huawei Ascend NPU' })
    }
    return npu.deviceType
  }, [npu.deviceType, t])

  if (npu.cores.length === 0) {
    return (
      <div className={`flex items-center justify-center text-[var(--text-muted)] ${className}`}>
        {t('npu.noData', { defaultValue: 'No NPU data available' })}
      </div>
    )
  }

  return (
    <div className={`space-y-4 ${className}`}>
      {/* 设备信息头 */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Cpu className="h-4 w-4 text-[var(--text-muted)]" />
          <span className="text-[13px] font-medium text-[var(--text-secondary)]">
            {deviceTypeLabel}
          </span>
        </div>

        {/* 负载均衡度 */}
        <div className="flex items-center gap-2">
          <span className="text-[11px] text-[var(--text-muted)]">
            {t('npu.balance', { defaultValue: 'Load Balance' })}:
          </span>
          <span className={`text-sm font-bold ${balanceClass}`}>{balance}%</span>
        </div>
      </div>

      {/* 核心使用率柱状图 */}
      <div className="flex items-end justify-center gap-2">
        {npu.cores.map((core) => (
          <CoreBar key={core.coreId} core={core} maxUtilization={maxUtilization} />
        ))}
      </div>

      {/* 核心详情卡片 */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        {npu.cores.map((core) => (
          <CoreCard key={core.coreId} core={core} />
        ))}
      </div>

      {/* 汇总信息 */}
      <div className="flex items-center justify-between rounded-lg bg-[var(--bg-secondary)] p-3">
        <div className="flex items-center gap-4">
          <div>
            <span className="text-[11px] text-[var(--text-muted)]">
              {t('npu.avgUsage', { defaultValue: 'Avg Usage' })}
            </span>
            <p className="text-sm font-bold text-[var(--text-primary)] tabular-nums">
              {Math.round(avgUtilization)}%
            </p>
          </div>
          <div>
            <span className="text-[11px] text-[var(--text-muted)]">
              {t('npu.memory', { defaultValue: 'Memory' })}
            </span>
            <p className="text-sm font-bold text-[var(--text-primary)] tabular-nums">
              {npu.usedMemoryMb} / {npu.totalMemoryMb} MB
            </p>
          </div>
          {npu.temperature && (
            <div>
              <span className="text-[11px] text-[var(--text-muted)]">
                {t('npu.temperature', { defaultValue: 'Temperature' })}
              </span>
              <p className="text-sm font-bold text-[var(--text-primary)] tabular-nums">
                {npu.temperature.toFixed(1)}°C
              </p>
            </div>
          )}
        </div>
        {npu.activeSessions > 0 && (
          <div>
            <span className="text-[11px] text-[var(--text-muted)]">
              {t('npu.sessions', { defaultValue: 'Sessions' })}
            </span>
            <p className="text-sm font-bold text-[var(--accent)] tabular-nums">
              {npu.activeSessions}
            </p>
          </div>
        )}
        {npu.inferenceCount > 0 && (
          <div>
            <span className="text-[11px] text-[var(--text-muted)]">
              {t('npu.inferences', { defaultValue: 'Inferences' })}
            </span>
            <p className="text-sm font-bold text-[var(--text-primary)] tabular-nums">
              {npu.inferenceCount.toLocaleString()}
            </p>
          </div>
        )}
      </div>
    </div>
  )
}
