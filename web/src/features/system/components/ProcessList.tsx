/**
 * 系统进程资源消耗排行组件（Top Processes）
 *
 * 显示占用 CPU/内存最高的前 5 个系统进程，便于运维快速定位异常与资源瓶颈。
 */

import { useTranslation } from 'react-i18next'
import { Terminal } from 'lucide-react'
import type { ProcessMetrics } from '../../../types/system'
import { getUsageColor } from './colors'

interface ProcessListProps {
  processes: ProcessMetrics[]
  className?: string
}

export function ProcessList({ processes, className = '' }: ProcessListProps) {
  const { t } = useTranslation('system')

  if (!processes || processes.length === 0) {
    return (
      <div
        className={`flex items-center justify-center py-6 text-[13px] text-[var(--text-muted)] ${className}`}
      >
        {t('process.noData', { defaultValue: 'No active process metrics available' })}
      </div>
    )
  }

  return (
    <div
      className={`overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] ${className}`}
    >
      <div className="flex items-center gap-2 border-b border-[var(--border)] bg-[var(--bg-secondary)] px-4 py-2.5">
        <Terminal className="h-4 w-4 text-[var(--text-muted)]" />
        <span className="text-[12px] font-medium text-[var(--text-secondary)]">
          {t('process.topList', { defaultValue: 'Top Resource Consumers' })}
        </span>
      </div>

      <div className="divide-y divide-[var(--border)]">
        <div className="grid grid-cols-12 px-4 py-2 text-[11px] font-medium text-[var(--text-muted)]">
          <span className="col-span-2">{t('process.pid', { defaultValue: 'PID' })}</span>
          <span className="col-span-5">{t('process.name', { defaultValue: 'Process Name' })}</span>
          <span className="col-span-3 text-right">
            {t('process.cpu', { defaultValue: 'CPU %' })}
          </span>
          <span className="col-span-2 text-right">
            {t('process.memory', { defaultValue: 'Memory' })}
          </span>
        </div>

        {processes.map((proc) => {
          const cpuColor = getUsageColor(proc.cpuPercent)
          return (
            <div
              key={proc.pid}
              className="grid grid-cols-12 items-center px-4 py-2.5 text-[12px] transition-colors hover:bg-[var(--bg-secondary)]"
            >
              <span className="col-span-2 font-mono text-[11px] text-[var(--text-muted)] tabular-nums">
                {proc.pid}
              </span>
              <span
                className="col-span-5 truncate font-medium text-[var(--text-primary)]"
                title={proc.name}
              >
                {proc.name}
              </span>
              <div
                className="col-span-3 flex items-center justify-end gap-1.5 font-bold tabular-nums"
                style={{ color: cpuColor }}
              >
                <span>{proc.cpuPercent.toFixed(1)}%</span>
              </div>
              <span className="col-span-2 text-right font-medium text-[var(--text-secondary)] tabular-nums">
                {proc.memoryMb} MB
              </span>
            </div>
          )
        })}
      </div>
    </div>
  )
}
