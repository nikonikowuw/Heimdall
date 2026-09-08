/**
 * CPU 核心热力图组件
 *
 * 显示每个 CPU 核心的使用率，使用颜色编码表示负载水平。
 * 支持实时更新和响应式布局。
 */

import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import type { CoreMetrics } from '../../../types/system'
import { getUsageColor, getUsageBgColor } from './colors'

interface CpuHeatmapProps {
  cores: CoreMetrics[]
  className?: string
}

function CoreCard({ core }: { core: CoreMetrics }) {
  const { t } = useTranslation('system')
  const color = getUsageColor(core.usagePercent)
  const bgColor = getUsageBgColor(core.usagePercent)

  return (
    <div
      className="flex flex-col items-center gap-2 rounded-xl border p-3 transition-all hover:scale-[1.02]"
      style={{
        borderColor: `color-mix(in srgb, ${color} 30%, transparent)`,
        backgroundColor: bgColor,
      }}
    >
      {/* 核心 ID */}
      <span className="text-[11px] font-medium text-[var(--text-muted)]">
        {t('cpu.core', { id: core.coreId, defaultValue: `Core ${core.coreId}` })}
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
            strokeDashoffset={150.8 * (1 - core.usagePercent / 100)}
            strokeLinecap="round"
            className="transition-all duration-500 ease-out"
          />
        </svg>
        <div className="absolute inset-0 flex items-center justify-center">
          <span className="text-xs font-bold tabular-nums" style={{ color }}>
            {Math.round(core.usagePercent)}%
          </span>
        </div>
      </div>

      {/* 频率信息 */}
      {core.frequencyMhz && (
        <span className="text-[10px] text-[var(--text-muted)] tabular-nums">
          {core.frequencyMhz} MHz
        </span>
      )}
    </div>
  )
}

export function CpuHeatmap({ cores, className = '' }: CpuHeatmapProps) {
  const { t } = useTranslation('system')

  // 按 coreId 稳定排序，防止轮询使用率波动导致卡片位置频繁跳动
  const sortedCores = useMemo(() => {
    return [...cores].sort((a, b) => a.coreId - b.coreId)
  }, [cores])

  if (cores.length === 0) {
    return (
      <div className={`flex items-center justify-center text-[var(--text-muted)] ${className}`}>
        {t('cpu.noData', { defaultValue: 'No CPU data available' })}
      </div>
    )
  }

  return (
    <div className={`grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 ${className}`}>
      {sortedCores.map((core) => (
        <CoreCard key={core.coreId} core={core} />
      ))}
    </div>
  )
}
