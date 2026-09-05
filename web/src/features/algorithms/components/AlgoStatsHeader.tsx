import React from 'react'
import { Box, Cpu, Layers, ShieldCheck, Sparkles } from 'lucide-react'
import { motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgorithmStats } from '@/types'

export interface AlgoStatsHeaderProps {
  stats: AlgorithmStats | null
  isLoading?: boolean
}

export const AlgoStatsHeader: React.FC<AlgoStatsHeaderProps> = ({ stats, isLoading }) => {
  const { t } = useTranslation('algo')

  const isMac =
    typeof navigator !== 'undefined' &&
    (navigator.platform.toUpperCase().indexOf('MAC') >= 0 ||
      navigator.userAgent.indexOf('Mac') >= 0)

  const platformLabel = isMac ? 'Apple Silicon (CoreML)' : 'Linux Embedded (NPU)'

  const statItems = [
    {
      id: 'total',
      label: t('stats.totalAlgorithms'),
      value: stats ? stats.totalAlgorithms : '-',
      icon: Cpu,
      color: 'text-[var(--accent)]',
      bgColor: 'bg-[var(--accent-soft)]',
    },
    {
      id: 'active',
      label: t('stats.activeVersions'),
      value: stats ? stats.totalActiveVersions : '-',
      icon: Layers,
      color: 'text-emerald-500',
      bgColor: 'bg-emerald-500/10',
    },
    {
      id: 'builtin',
      label: t('stats.builtinModels'),
      value: stats ? stats.builtinAlgorithms : '-',
      icon: ShieldCheck,
      color: 'text-indigo-400',
      bgColor: 'bg-indigo-500/10',
    },
    {
      id: 'custom',
      label: t('stats.customModels'),
      value: stats ? stats.customAlgorithms : '-',
      icon: Box,
      color: 'text-amber-400',
      bgColor: 'bg-amber-500/10',
    },
  ]

  return (
    <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-5">
      {statItems.map((item, index) => {
        const Icon = item.icon
        return (
          <motion.div
            key={item.id}
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{
              duration: motionTokens.duration.normal,
              delay: index * 0.05,
              ease: motionTokens.easing.smooth,
            }}
            className="frosted-glass flex items-center justify-between rounded-2xl border border-[var(--border)] p-4 transition-all duration-200 hover:border-[var(--border-strong)]"
          >
            <div className="space-y-1">
              <span className="text-xs font-medium text-[var(--text-muted)]">{item.label}</span>
              <div className="flex items-baseline gap-1.5">
                <span className="font-mono text-2xl font-bold tracking-tight text-[var(--text-primary)]">
                  {isLoading ? '...' : item.value}
                </span>
              </div>
            </div>
            <div
              className={`flex h-11 w-11 shrink-0 items-center justify-center rounded-xl ${item.bgColor} ${item.color}`}
            >
              <Icon className="h-5 w-5" />
            </div>
          </motion.div>
        )
      })}

      {/* 平台架构识别卡片 */}
      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{
          duration: motionTokens.duration.normal,
          delay: 0.2,
          ease: motionTokens.easing.smooth,
        }}
        className="frosted-glass flex items-center justify-between rounded-2xl border border-[var(--border)] p-4 transition-all duration-200 hover:border-[var(--border-strong)]"
      >
        <div className="space-y-1">
          <span className="text-xs font-medium text-[var(--text-muted)]">
            {t('stats.platformArch')}
          </span>
          <div className="flex items-center gap-1.5">
            <span className="font-mono text-xs font-semibold text-emerald-400">
              {platformLabel}
            </span>
          </div>
        </div>
        <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-purple-500/10 text-purple-400">
          <Sparkles className="h-5 w-5" />
        </div>
      </motion.div>
    </div>
  )
}
