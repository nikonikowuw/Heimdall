/**
 * 内存详细统计组件
 *
 * 显示详细的内存使用情况，包括总量、已用、可用、缓存和 Buffer。
 * 使用进度条和分类显示内存分配。
 */

import { useTranslation } from 'react-i18next'
import type { MemoryMetrics } from '../../../types/system'

interface MemoryDetailProps {
  memory: MemoryMetrics
  className?: string
}

function formatMb(mb: number): string {
  if (mb >= 1024) {
    return `${(mb / 1024).toFixed(1)} GB`
  }
  return `${mb} MB`
}

function MemoryBar({ memory }: { memory: MemoryMetrics }) {
  const { t } = useTranslation('system')
  const usedPercent = memory.totalMb > 0 ? (memory.usedMb / memory.totalMb) * 100 : 0
  // Cached 和 Buffer 包含在 Used 内，按从底到顶的 z-index 分层显示
  const cachedPercent = memory.totalMb > 0 ? (memory.cachedMb / memory.totalMb) * 100 : 0
  const bufferPercent = memory.totalMb > 0 ? (memory.bufferMb / memory.totalMb) * 100 : 0

  return (
    <div className="space-y-2">
      {/* 总体使用率 */}
      <div className="flex items-center justify-between text-[13px]">
        <span className="text-[var(--text-secondary)]">
          {t('memoryMetrics.usage', { defaultValue: 'Memory Usage' })}
        </span>
        <span className="font-bold text-[var(--text-primary)] tabular-nums">
          {Math.round(usedPercent)}%
        </span>
      </div>

      {/* 进度条 — Used 为底层，Cached 和 Buffer 为上层叠加（均包含在 Used 内） */}
      <div className="relative h-4 overflow-hidden rounded-full bg-[var(--bg-secondary)]">
        <div
          className="absolute top-0 left-0 h-full rounded-full bg-[var(--accent)] transition-all duration-500"
          style={{ width: `${usedPercent}%` }}
        />
        <div
          className="absolute top-0 left-0 h-full rounded-full bg-[var(--accent-green)] transition-all duration-500"
          style={{ width: `${cachedPercent}%`, opacity: 0.45 }}
        />
        <div
          className="absolute top-0 left-0 h-full rounded-full bg-[var(--accent-amber)] transition-all duration-500"
          style={{ width: `${bufferPercent}%`, opacity: 0.6 }}
        />
      </div>

      {/* 图例 */}
      <div className="flex flex-wrap gap-4 text-[11px]">
        <div className="flex items-center gap-1.5">
          <div className="h-2 w-2 rounded-full bg-[var(--accent)]" />
          <span className="text-[var(--text-muted)]">
            {t('memoryMetrics.used', { defaultValue: 'Used' })}
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          <div
            className="h-2 w-2 rounded-full bg-[var(--accent-green)]"
            style={{ opacity: 0.45 }}
          />
          <span className="text-[var(--text-muted)]">
            {t('memoryMetrics.cached', { defaultValue: 'Cached (reclaimable)' })}
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          <div className="h-2 w-2 rounded-full bg-[var(--accent-amber)]" style={{ opacity: 0.6 }} />
          <span className="text-[var(--text-muted)]">
            {t('memoryMetrics.buffer', { defaultValue: 'Buffer' })}
          </span>
        </div>
      </div>
    </div>
  )
}

function MemoryStats({ memory }: { memory: MemoryMetrics }) {
  const { t } = useTranslation('system')
  const stats = [
    {
      label: t('memoryMetrics.total', { defaultValue: 'Total' }),
      value: formatMb(memory.totalMb),
      color: 'var(--text-primary)',
    },
    {
      label: t('memoryMetrics.used', { defaultValue: 'Used' }),
      value: formatMb(memory.usedMb),
      color: 'var(--accent)',
    },
    {
      label: t('memoryMetrics.available', { defaultValue: 'Available' }),
      value: formatMb(memory.availableMb),
      color: 'var(--accent-green)',
    },
    {
      label: t('memoryMetrics.cached', { defaultValue: 'Cached' }),
      value: formatMb(memory.cachedMb),
      color: 'var(--accent-green)',
    },
    {
      label: t('memoryMetrics.buffer', { defaultValue: 'Buffer' }),
      value: formatMb(memory.bufferMb),
      color: 'var(--accent-amber)',
    },
  ]

  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
      {stats.map((stat) => (
        <div
          key={stat.label}
          className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] p-3"
        >
          <p className="text-[11px] text-[var(--text-muted)]">{stat.label}</p>
          <p className="text-sm font-bold tabular-nums" style={{ color: stat.color }}>
            {stat.value}
          </p>
        </div>
      ))}
    </div>
  )
}

function SwapInfo({ memory }: { memory: MemoryMetrics }) {
  const { t } = useTranslation('system')

  if (memory.swapTotalMb === 0) {
    return null
  }

  const swapUsedPercent =
    memory.swapTotalMb > 0 ? (memory.swapUsedMb / memory.swapTotalMb) * 100 : 0

  return (
    <div className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] p-3">
      <div className="flex items-center justify-between text-[13px]">
        <span className="text-[var(--text-secondary)]">
          {t('memoryMetrics.swap', { defaultValue: 'Swap' })}
        </span>
        <span className="font-bold text-[var(--text-primary)] tabular-nums">
          {formatMb(memory.swapUsedMb)} / {formatMb(memory.swapTotalMb)}
        </span>
      </div>
      <div className="mt-2 h-2 overflow-hidden rounded-full bg-[var(--bg-secondary)]">
        <div
          className="h-full rounded-full bg-[var(--accent-amber)] transition-all duration-500"
          style={{ width: `${swapUsedPercent}%` }}
        />
      </div>
    </div>
  )
}

export function MemoryDetail({ memory, className = '' }: MemoryDetailProps) {
  return (
    <div className={`space-y-4 ${className}`}>
      {/* 内存使用率条 */}
      <MemoryBar memory={memory} />

      {/* 详细统计 */}
      <MemoryStats memory={memory} />

      {/* Swap 信息 */}
      <SwapInfo memory={memory} />
    </div>
  )
}
