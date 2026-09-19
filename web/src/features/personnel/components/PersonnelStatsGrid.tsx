import React from 'react'
import { AlertCircle, Gauge, Layers, Users } from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '../../../lib/motionTokens'
import type { PersonnelStats } from '../../../types'

export interface PersonnelStatsGridProps {
  stats: PersonnelStats
  /** 是否已拿到过至少一次有效统计；为 false 时数值显示占位符 */
  hasData: boolean
  /** 最近一次统计请求失败说明；null 表示正常。失败时保留上次数值并旁挂提示 */
  error: string | null
}

interface StatCard {
  id: string
  label: string
  value: string
  /** 数值单位后缀，如「张 / 人」 */
  suffix?: string
  icon: typeof Users
  iconClass: string
  iconBgClass: string
}

/**
 * 底库规模统计。
 *
 * 算法就绪状态不在这里占格：文案较长（"算法未就绪 (请部署人脸包)"）会挤爆
 * 数字卡，且它同时是重提按钮的可用性依据，放在操作区旁边更贴近决策点。
 */
export function PersonnelStatsGrid({
  stats,
  hasData,
  error,
}: PersonnelStatsGridProps): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])
  const reduceMotion = useReducedMotion()

  // 未取到过数据时回落为占位符：绝不把「未知」渲染成 0，否则运维会误读为空底库。
  // 刷新失败时保留上次数值，由 error 提示说明数据可能过期，避免每次检索都闪一遍占位符
  const showValue = hasData
  const placeholder = '—'
  const avgSamples =
    stats.totalPersonnel > 0 ? (stats.totalFaces / stats.totalPersonnel).toFixed(1) : null

  const cards: StatCard[] = [
    {
      id: 'personnel',
      label: t('stats.totalPersonnel'),
      value: showValue ? String(stats.totalPersonnel) : placeholder,
      icon: Users,
      iconClass: 'text-emerald-500',
      iconBgClass: 'bg-emerald-500/10',
    },
    {
      id: 'faces',
      label: t('stats.totalFaces'),
      value: showValue ? String(stats.totalFaces) : placeholder,
      icon: Layers,
      iconClass: 'text-cyan-500',
      iconBgClass: 'bg-cyan-500/10',
    },
    {
      id: 'coverage',
      label: t('stats.avgSamples'),
      value: showValue && avgSamples !== null ? avgSamples : placeholder,
      suffix: showValue && avgSamples !== null ? t('stats.avgSamplesUnit') : undefined,
      icon: Gauge,
      iconClass: 'text-[var(--accent)]',
      iconBgClass: 'bg-[var(--accent-soft)]',
    },
  ]

  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-3 lg:gap-4">
        {cards.map((card, index) => {
          const Icon = card.icon
          return (
            <motion.div
              key={card.id}
              initial={reduceMotion ? false : { opacity: 0, y: motionTokens.distance.sm }}
              animate={{ opacity: 1, y: 0 }}
              transition={{
                duration: motionTokens.duration.normal,
                delay: reduceMotion ? 0 : index * 0.05,
                ease: motionTokens.easing.smooth,
              }}
              className="frosted-glass flex items-center justify-between gap-3 rounded-2xl p-3.5 sm:p-4"
            >
              <div className="min-w-0 space-y-1">
                <span className="block truncate text-[11px] font-medium text-[var(--text-muted)] sm:text-xs">
                  {card.label}
                </span>
                <span className="flex items-baseline gap-1.5">
                  <span className="font-data block text-xl font-bold tracking-tight text-[var(--text-primary)] tabular-nums sm:text-2xl">
                    {card.value}
                  </span>
                  {card.suffix !== undefined && (
                    <span className="text-[10px] font-medium text-[var(--text-muted)]">
                      {card.suffix}
                    </span>
                  )}
                </span>
              </div>
              <div
                className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-xl sm:h-11 sm:w-11 ${card.iconBgClass} ${card.iconClass}`}
              >
                <Icon className="h-5 w-5" />
              </div>
            </motion.div>
          )
        })}
      </div>

      {error !== null && (
        <p
          role="status"
          className="flex items-center gap-1.5 text-[11px] text-[var(--accent-amber)]"
        >
          <AlertCircle className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          {error}
        </p>
      )}
    </div>
  )
}
