import React from 'react'
import { Box, Cpu, Layers, ShieldCheck } from 'lucide-react'
import { motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgorithmStats } from '@/types'

export interface AlgoStatsHeaderProps {
  stats: AlgorithmStats | null
  /** 统计与筛选无关，只反映自身加载状态，不随关键字输入闪烁 */
  isLoading?: boolean
  /** 统计请求失败时的提示；为空串表示未知错误 */
  error?: string | null
}

/**
 * 仓库统计指标。
 *
 * 仅承载统计量，宿主平台只在设备维度出现一次（见 `HostPlatformBadge`），
 * 不再把设备信息混排进同一网格，避免窄屏下出现孤行与语义混淆。
 */
export function AlgoStatsHeader({
  stats,
  isLoading,
  error,
}: AlgoStatsHeaderProps): React.ReactElement {
  const { t } = useTranslation('algo')

  const statItems = [
    {
      id: 'total',
      label: t('stats.totalAlgorithms'),
      value: stats?.totalAlgorithms,
      icon: Cpu,
      color: 'text-[var(--accent)]',
      bgColor: 'bg-[var(--accent-soft)]',
    },
    {
      id: 'active',
      label: t('stats.activeVersions'),
      value: stats?.totalActiveVersions,
      icon: Layers,
      color: 'text-emerald-500',
      bgColor: 'bg-emerald-500/10',
    },
    {
      id: 'builtin',
      label: t('stats.builtinModels'),
      value: stats?.builtinAlgorithms,
      icon: ShieldCheck,
      color: 'text-indigo-400',
      bgColor: 'bg-indigo-500/10',
    },
    {
      id: 'custom',
      label: t('stats.customModels'),
      value: stats?.customAlgorithms,
      icon: Box,
      color: 'text-amber-400',
      bgColor: 'bg-amber-500/10',
    },
  ]

  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        {statItems.map((item, index) => {
          const Icon = item.icon
          const value = isLoading ? null : item.value
          return (
            <motion.div
              key={item.id}
              initial={{ opacity: 0, y: motionTokens.distance.md }}
              animate={{ opacity: 1, y: 0 }}
              transition={{
                duration: motionTokens.duration.normal,
                delay: index * 0.05,
                ease: motionTokens.easing.smooth,
              }}
              className="frosted-glass flex items-center justify-between gap-3 rounded-2xl border border-[var(--border)] p-4 transition-colors duration-200 hover:border-[var(--border-strong)]"
            >
              <div className="min-w-0 space-y-1">
                <span className="block truncate text-xs font-medium text-[var(--text-muted)]">
                  {item.label}
                </span>
                <span className="font-data block text-2xl font-bold tracking-tight text-[var(--text-primary)] tabular-nums">
                  {value === undefined || value === null ? '—' : value}
                </span>
              </div>
              <div
                className={`flex h-11 w-11 shrink-0 items-center justify-center rounded-xl ${item.bgColor} ${item.color}`}
              >
                <Icon className="h-5 w-5" />
              </div>
            </motion.div>
          )
        })}
      </div>

      {error !== null && error !== undefined && (
        <p role="status" className="text-[11px] text-[var(--accent-amber)]">
          {error || t('stats.loadFailed')}
        </p>
      )}
    </div>
  )
}
