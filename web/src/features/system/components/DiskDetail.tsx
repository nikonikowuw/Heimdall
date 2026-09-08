/**
 * 磁盘详细统计组件
 *
 * 显示详细的磁盘使用情况，包括容量和 Inode 统计。
 * 使用进度条和分类显示磁盘分配。
 */

import { useTranslation } from 'react-i18next'
import type { DiskMetrics } from '../../../types/system'

interface DiskDetailProps {
  disk: DiskMetrics
  className?: string
}

function formatGb(gb: number): string {
  return `${gb.toFixed(1)} GB`
}

function formatInode(inode: number): string {
  if (inode >= 1000000) {
    return `${(inode / 1000000).toFixed(1)}M`
  }
  if (inode >= 1000) {
    return `${(inode / 1000).toFixed(1)}K`
  }
  return inode.toString()
}

function DiskBar({ disk }: { disk: DiskMetrics }) {
  const { t } = useTranslation('system')
  const usedPercent = disk.totalGb > 0 ? (disk.usedGb / disk.totalGb) * 100 : 0

  return (
    <div className="space-y-2">
      {/* 总体使用率 */}
      <div className="flex items-center justify-between text-[13px]">
        <span className="text-[var(--text-secondary)]">
          {t('diskMetrics.usage', { defaultValue: 'Disk Usage' })}
        </span>
        <span className="font-bold text-[var(--text-primary)] tabular-nums">
          {Math.round(usedPercent)}%
        </span>
      </div>

      {/* 进度条 */}
      <div className="relative h-4 overflow-hidden rounded-full bg-[var(--bg-secondary)]">
        <div
          className="absolute top-0 left-0 h-full rounded-full bg-[var(--accent)] transition-all duration-500"
          style={{ width: `${usedPercent}%` }}
        />
      </div>

      {/* 容量信息 */}
      <div className="flex items-center justify-between text-[11px]">
        <span className="text-[var(--text-muted)]">
          {t('diskMetrics.usedAmount', {
            amount: formatGb(disk.usedGb),
            defaultValue: `${formatGb(disk.usedGb)} used`,
          })}
        </span>
        <span className="text-[var(--text-muted)]">
          {t('diskMetrics.freeAmount', {
            amount: formatGb(disk.availableGb),
            defaultValue: `${formatGb(disk.availableGb)} free`,
          })}
        </span>
      </div>
    </div>
  )
}

function InodeStats({ disk }: { disk: DiskMetrics }) {
  const { t } = useTranslation('system')

  if (disk.inodeTotal === 0) {
    return null
  }

  const usedPercent = disk.inodeTotal > 0 ? (disk.inodeUsed / disk.inodeTotal) * 100 : 0

  return (
    <div className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] p-3">
      <div className="flex items-center justify-between text-[13px]">
        <span className="text-[var(--text-secondary)]">
          {t('diskMetrics.inode', { defaultValue: 'Inode' })}
        </span>
        <span className="font-bold text-[var(--text-primary)] tabular-nums">
          {Math.round(usedPercent)}%
        </span>
      </div>

      <div className="mt-2 h-2 overflow-hidden rounded-full bg-[var(--bg-secondary)]">
        <div
          className="h-full rounded-full bg-[var(--accent-green)] transition-all duration-500"
          style={{ width: `${usedPercent}%` }}
        />
      </div>

      <div className="mt-2 flex items-center justify-between text-[11px]">
        <span className="text-[var(--text-muted)]">
          {t('diskMetrics.inodeUsed', {
            count: formatInode(disk.inodeUsed),
            defaultValue: `${formatInode(disk.inodeUsed)} used`,
          })}
        </span>
        <span className="text-[var(--text-muted)]">
          {t('diskMetrics.inodeFree', {
            count: formatInode(disk.inodeAvailable),
            defaultValue: `${formatInode(disk.inodeAvailable)} free`,
          })}
        </span>
      </div>
    </div>
  )
}

export function DiskDetail({ disk, className = '' }: DiskDetailProps) {
  const { t } = useTranslation('system')

  return (
    <div className={`space-y-4 ${className}`}>
      {/* 磁盘使用率条 */}
      <DiskBar disk={disk} />

      {/* 容量统计 */}
      <div className="grid grid-cols-2 gap-3">
        <div className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] p-3">
          <p className="text-[11px] text-[var(--text-muted)]">
            {t('diskMetrics.total', { defaultValue: 'Total' })}
          </p>
          <p className="text-sm font-bold text-[var(--text-primary)] tabular-nums">
            {formatGb(disk.totalGb)}
          </p>
        </div>
        <div className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] p-3">
          <p className="text-[11px] text-[var(--text-muted)]">
            {t('diskMetrics.available', { defaultValue: 'Available' })}
          </p>
          <p className="text-sm font-bold text-[var(--accent-green)] tabular-nums">
            {formatGb(disk.availableGb)}
          </p>
        </div>
      </div>

      {/* Inode 统计 */}
      <InodeStats disk={disk} />
    </div>
  )
}
