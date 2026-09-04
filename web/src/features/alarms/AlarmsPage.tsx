import React from 'react'
import { AlertCircle, Filter, RefreshCw, Search } from 'lucide-react'
import { useTranslation } from 'react-i18next'

export const AlarmsPage: React.FC = () => {
  const { t } = useTranslation('alarm')
  const { t: tc } = useTranslation('common')

  return (
    <div className="flex h-full flex-col gap-4">
      {/* 顶部搜索与过滤工具栏 */}
      <div className="frosted-glass flex items-center justify-between rounded-xl p-3">
        <div className="flex items-center gap-2">
          <AlertCircle className="h-5 w-5 text-[var(--destructive)]" />
          <span className="font-semibold text-[var(--text-primary)]">{t('title')}</span>
        </div>

        <div className="flex items-center gap-2">
          <div className="flex items-center gap-2 rounded-lg border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-1.5 text-xs text-[var(--text-secondary)]">
            <Search className="h-3.5 w-3.5 text-[var(--text-muted)]" />
            <input
              type="text"
              placeholder={t('searchPlaceholder')}
              className="bg-transparent outline-none placeholder:text-[var(--text-muted)]"
            />
          </div>

          <button className="flex items-center gap-1.5 rounded-lg border border-[var(--border)] px-3 py-1.5 text-xs font-medium text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]">
            <Filter className="h-3.5 w-3.5" />
            {tc('actions.filter')}
          </button>
          <button className="flex items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3 py-1.5 text-xs font-medium text-white hover:opacity-90">
            <RefreshCw className="h-3.5 w-3.5" />
            {tc('actions.refresh')}
          </button>
        </div>
      </div>

      {/* 告警事件数据表格 */}
      <div className="frosted-glass flex-1 overflow-auto rounded-xl">
        <table className="w-full text-left text-xs">
          <thead className="border-b border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)] uppercase">
            <tr>
              <th className="px-4 py-3 font-medium">{t('columns.eventId')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.camera')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.alarmType')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.targetLabel')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.confidence')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.occurredAt')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.evidence')}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-[var(--border)] text-[var(--text-secondary)]">
            <tr className="transition-colors hover:bg-[var(--accent-soft)]/50">
              <td className="px-4 py-3 font-mono text-[var(--accent)]">EV-20250518-001</td>
              <td className="px-4 py-3 font-mono">CAM-01</td>
              <td className="px-4 py-3">
                <span className="rounded bg-red-500/10 px-2 py-0.5 text-[11px] font-medium text-red-600 dark:text-red-400">
                  {t('types.lineCrossing')}
                </span>
              </td>
              <td className="px-4 py-3">person</td>
              <td className="px-4 py-3 font-mono tabular-nums">92.4%</td>
              <td className="px-4 py-3 font-mono tabular-nums">2025-05-18 14:20:05</td>
              <td className="px-4 py-3">
                <span className="cursor-pointer text-[var(--accent)] hover:underline">
                  {t('viewImage')}
                </span>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  )
}
