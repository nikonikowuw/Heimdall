import React from 'react'
import { FileText, RefreshCw } from 'lucide-react'
import { useTranslation } from 'react-i18next'

export const OplogPage: React.FC = () => {
  const { t } = useTranslation('oplog')

  return (
    <div className="flex h-full flex-col gap-4">
      <div className="frosted-glass flex items-center justify-between rounded-xl p-3">
        <div className="flex items-center gap-2">
          <FileText className="h-5 w-5 text-[var(--accent)]" />
          <span className="font-semibold text-[var(--text-primary)]">{t('title')}</span>
        </div>

        <button className="flex items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3 py-1.5 text-xs font-medium text-white hover:opacity-90">
          <RefreshCw className="h-3.5 w-3.5" />
          {t('refreshStream')}
        </button>
      </div>

      <div className="frosted-glass flex-1 overflow-auto rounded-xl">
        <table className="w-full text-left text-xs">
          <thead className="border-b border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)] uppercase">
            <tr>
              <th className="px-4 py-3 font-medium">{t('columns.username')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.module')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.action')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.method')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.path')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.ip')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.status')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.duration')}</th>
              <th className="px-4 py-3 font-medium">{t('columns.time')}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-[var(--border)] text-[var(--text-secondary)]">
            <tr className="transition-colors hover:bg-[var(--accent-soft)]/50">
              <td className="px-4 py-3 font-medium text-[var(--text-primary)]">admin</td>
              <td className="px-4 py-3 font-mono">task</td>
              <td className="px-4 py-3">update_rules</td>
              <td className="px-4 py-3 font-mono font-semibold text-blue-600 dark:text-blue-400">
                PUT
              </td>
              <td className="px-4 py-3 font-mono">/api/v1/tasks/CAM-01/rules</td>
              <td className="px-4 py-3 font-mono">127.0.0.1</td>
              <td className="px-4 py-3 font-mono text-emerald-600 dark:text-emerald-400">200 OK</td>
              <td className="px-4 py-3 font-mono tabular-nums">4 ms</td>
              <td className="px-4 py-3 font-mono tabular-nums">2025-05-18 14:15:32</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  )
}
