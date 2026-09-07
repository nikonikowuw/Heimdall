import { useState, type ReactElement } from 'react'
import { ChevronDown, FileText, RefreshCw } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { formatTimestamp } from '../../lib/time'
import { useOplogs } from './hooks/useOplogs'

const MODULE_FILTERS = [
  'all',
  'auth',
  'camera',
  'task',
  'task_instance',
  'algorithm',
  'alarm',
  'evidence',
  'system',
] as const

type ModuleFilter = (typeof MODULE_FILTERS)[number]

type StatusTone = 'success' | 'clientError' | 'serverError' | 'other'

function getStatusTone(statusCode: number): StatusTone {
  if (statusCode >= 200 && statusCode < 300) return 'success'
  if (statusCode >= 400 && statusCode < 500) return 'clientError'
  if (statusCode >= 500) return 'serverError'
  return 'other'
}

function getStatusClassName(statusCode: number): string {
  switch (getStatusTone(statusCode)) {
    case 'success':
      return 'text-[var(--accent-green)]'
    case 'clientError':
      return 'text-[var(--accent-amber)]'
    case 'serverError':
      return 'text-[var(--destructive)]'
    case 'other':
      return 'text-[var(--text-secondary)]'
  }
}

function isModuleFilter(value: string): value is ModuleFilter {
  return MODULE_FILTERS.some((module) => module === value)
}

export function OplogPage(): ReactElement {
  const { t, i18n } = useTranslation('oplog')
  const [moduleFilter, setModuleFilter] = useState<ModuleFilter>('all')
  const { logs, isLoading, isLoadingMore, hasMore, error, refresh, loadMore } = useOplogs(
    moduleFilter === 'all' ? '' : moduleFilter,
  )

  return (
    <div className="flex h-full min-h-0 flex-col gap-4">
      <section className="frosted-glass flex flex-wrap items-center justify-between gap-3 rounded-2xl p-3.5">
        <div className="flex min-w-0 items-center gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
            <FileText className="h-5 w-5" strokeWidth={1.75} />
          </div>
          <div className="min-w-0">
            <h2 className="truncate text-sm font-semibold text-[var(--text-primary)]">
              {t('title')}
            </h2>
            <p className="font-data mt-0.5 text-[10px] tracking-[0.12em] text-[var(--text-muted)] uppercase">
              {t('subtitle')}
            </p>
          </div>
        </div>

        <div className="flex min-w-0 items-center gap-2.5">
          <label
            htmlFor="oplog-module-filter"
            className="shrink-0 text-xs font-medium text-[var(--text-muted)]"
          >
            {t('filterModule')}
          </label>
          <select
            id="oplog-module-filter"
            value={moduleFilter}
            onChange={(event) => {
              const value = event.target.value
              if (isModuleFilter(value)) {
                setModuleFilter(value)
              }
            }}
            className="h-8 min-w-32 rounded-lg border border-[var(--border-strong)] bg-[var(--bg-surface-solid)] px-2.5 text-xs text-[var(--text-primary)] transition-colors outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--ring)]"
          >
            {MODULE_FILTERS.map((module) => (
              <option key={module} value={module}>
                {t(`modules.${module}`)}
              </option>
            ))}
          </select>
          <button
            type="button"
            onClick={refresh}
            disabled={isLoading}
            aria-label={t('refreshStream')}
            title={t('refreshStream')}
            className="reticle-target flex h-8 shrink-0 items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3 text-xs font-medium text-white transition-opacity hover:opacity-90 disabled:cursor-wait disabled:opacity-60"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? 'animate-spin' : ''}`} />
            <span>{t('refreshStream')}</span>
          </button>
        </div>
      </section>

      {error && (
        <div
          className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-[var(--destructive)]/30 bg-[var(--destructive)]/8 px-3.5 py-2.5 text-xs text-[var(--destructive)]"
          role="alert"
        >
          <span>{t('loadError', { error: error || t('unknownError') })}</span>
          <button
            type="button"
            onClick={refresh}
            className="reticle-target inline-flex items-center gap-1 rounded-md px-2 py-1 font-medium transition-colors hover:bg-[var(--destructive)]/10"
          >
            <RefreshCw className="h-3 w-3" />
            {t('retry')}
          </button>
        </div>
      )}

      <section className="frosted-glass min-h-0 flex-1 overflow-hidden rounded-2xl">
        <div className="h-full overflow-auto">
          <table className="w-full min-w-[960px] text-left text-xs">
            <thead className="sticky top-0 z-10 border-b border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)] uppercase">
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
              {isLoading ? (
                <tr>
                  <td colSpan={9} className="px-4 py-12 text-center text-[var(--text-muted)]">
                    {t('loading')}
                  </td>
                </tr>
              ) : logs.length === 0 ? (
                <tr>
                  <td colSpan={9} className="px-4 py-12 text-center text-[var(--text-muted)]">
                    {t('empty')}
                  </td>
                </tr>
              ) : (
                logs.map((log) => (
                  <tr key={log.id} className="transition-colors hover:bg-[var(--accent-soft)]/50">
                    <td className="px-4 py-3 font-medium text-[var(--text-primary)]">
                      {log.username || t('unknown')}
                    </td>
                    <td className="font-data px-4 py-3 text-[var(--text-primary)]">{log.module}</td>
                    <td className="px-4 py-3">{log.action}</td>
                    <td className="font-data px-4 py-3 font-semibold text-[var(--accent)]">
                      {log.method}
                    </td>
                    <td className="font-data max-w-[280px] px-4 py-3" title={log.path}>
                      <span className="block truncate">{log.path}</span>
                    </td>
                    <td className="font-data px-4 py-3">{log.ip || t('unknown')}</td>
                    <td
                      className={`font-data px-4 py-3 font-semibold ${getStatusClassName(log.statusCode)}`}
                    >
                      {t(`status.${getStatusTone(log.statusCode)}`, { code: log.statusCode })}
                    </td>
                    <td className="font-data px-4 py-3 tabular-nums">
                      {t('durationValue', { value: log.durationMs })}
                    </td>
                    <td className="font-data px-4 py-3 whitespace-nowrap text-[var(--text-muted)] tabular-nums">
                      {formatTimestamp(log.createdAt, i18n.language)}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </section>

      {!isLoading && logs.length > 0 && (
        <div className="flex min-h-8 items-center justify-center">
          {hasMore ? (
            <button
              type="button"
              onClick={loadMore}
              disabled={isLoadingMore}
              className="reticle-target inline-flex items-center gap-1.5 rounded-lg border border-[var(--border-strong)] px-3 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)] disabled:cursor-wait disabled:opacity-60"
            >
              <ChevronDown className="h-3.5 w-3.5" />
              {isLoadingMore ? t('loadingMore') : t('loadMore')}
            </button>
          ) : (
            <span className="text-[11px] text-[var(--text-muted)]">{t('noMore')}</span>
          )}
        </div>
      )}
    </div>
  )
}
