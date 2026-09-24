import { useEffect, useMemo, useRef, useState, type ReactElement } from 'react'
import { Filter, Layers, RefreshCw, RotateCcw, Search, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { DateTimeRangePicker } from '@/components/DateTimeRangePicker'
import { resolveEffectiveTimeRange, type DateTimeRangeValue } from '@/lib/dateRange'
import { useDebounce } from '@/hooks/use-debounce'
import { formatTimestamp } from '@/lib/time'
import { LogDetailDrawer, type InspectableLog } from './components/LogDetailDrawer'
import { LogEmptyState } from './components/LogEmptyState'
import { LogPaginationBar } from './components/LogPaginationBar'
import { LogTableSkeleton } from './components/LogTableSkeleton'
import { OperationStatsCards } from './components/OperationStatsCards'
import { OplogToolbar, type LogDensity } from './components/OplogToolbar'
import { useOplogs } from './hooks/useOplogs'
import { isMemberOf, MODULE_FILTERS, type ModuleFilter, type StatusFilter } from './logFilters'
import {
  classifyHttpStatus,
  getToneClasses,
  latencyTone,
  methodTone,
  statusTone,
  type LogTone,
} from './logTone'
import { DEFAULT_LOG_PAGE_SIZE } from './logPaging'

/** 状态码分段筛选；tone 只用于选中态的实心配色 */
const STATUS_SEGMENTS: readonly { id: StatusFilter; labelKey: string; tone: LogTone }[] = [
  { id: 'all', labelKey: 'toolbar.filterAllStatuses', tone: 'accent' },
  { id: 'success', labelKey: 'toolbar.filterSuccess', tone: 'success' },
  { id: 'failed', labelKey: 'toolbar.filterFailedOnly', tone: 'danger' },
]

export function OperationLogsTab(): ReactElement {
  const { t, i18n } = useTranslation('oplog')
  const { t: tAlarm } = useTranslation('alarm')

  // 分页控制
  const [page, setPage] = useState<number>(1)
  const [pageSize, setPageSize] = useState<number>(DEFAULT_LOG_PAGE_SIZE)
  const tableContainerRef = useRef<HTMLDivElement>(null)

  // 时间范围筛选
  const [timeRange, setTimeRange] = useState<DateTimeRangeValue>({ quickPreset: 'all' })
  const effectiveTimeRange = useMemo(() => resolveEffectiveTimeRange(timeRange), [timeRange])

  // 服务端筛选条件：全部下推，前端不再对已取回的一页做二次过滤
  const [moduleFilter, setModuleFilter] = useState<ModuleFilter>('all')
  const [statusFilter, setStatusFilter] = useState<StatusFilter>('all')
  const [searchInput, setSearchInput] = useState('')
  const debouncedSearch = useDebounce(searchInput.trim(), 250)

  // 关键词变动重置回第 1 页
  useEffect(() => {
    setPage(1)
  }, [debouncedSearch])

  // 显示控制
  const [density, setDensity] = useState<LogDensity>('comfortable')
  const [inspectingLog, setInspectingLog] = useState<InspectableLog | null>(null)

  const { logs, isLoading, hasMore, error, refresh } = useOplogs(
    {
      module: moduleFilter,
      status: statusFilter,
      keyword: debouncedSearch,
      fromMs: effectiveTimeRange.startTime,
      toMs: effectiveTimeRange.endTime,
    },
    page,
    pageSize,
  )

  // 翻页平滑回顶
  const handlePageChange = (newPage: number) => {
    setPage(newPage)
    tableContainerRef.current?.scrollTo({ top: 0, behavior: 'smooth' })
  }

  const handlePageSizeChange = (newSize: number) => {
    setPageSize(newSize)
    setPage(1)
    tableContainerRef.current?.scrollTo({ top: 0, behavior: 'smooth' })
  }

  const isFiltered =
    moduleFilter !== 'all' ||
    statusFilter !== 'all' ||
    timeRange.quickPreset !== 'all' ||
    searchInput.trim().length > 0

  function handleResetFilters(): void {
    setModuleFilter('all')
    setStatusFilter('all')
    setTimeRange({ quickPreset: 'all' })
    setSearchInput('')
    setPage(1)
  }

  const rowPaddingClass = density === 'compact' ? 'py-2' : 'py-3'

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col gap-3">
      {/* 顶部统计指标卡片（口径为当前页） */}
      <OperationStatsCards
        logs={logs}
        statusFilter={statusFilter}
        onSelectStatusFilter={(filter) => {
          setStatusFilter(filter)
          setPage(1)
        }}
      />

      {/* 现代智能控制栏 */}
      <section className="frosted-glass flex flex-wrap items-center justify-between gap-2.5 rounded-xl p-2.5">
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          {/* 服务端关键字检索 */}
          <div className="relative max-w-xs min-w-48 flex-1">
            <Search className="absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)]" />
            <input
              type="text"
              value={searchInput}
              onChange={(e) => setSearchInput(e.target.value)}
              placeholder={t('toolbar.searchOperations')}
              aria-label={t('toolbar.searchOperations')}
              className="h-8 w-full rounded-lg border border-[var(--border-strong)] bg-[var(--bg-surface-solid)] pr-7 pl-8 text-xs text-[var(--text-primary)] transition-colors outline-none placeholder:text-[var(--text-muted)] focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--ring)]"
            />
            {searchInput && (
              <button
                type="button"
                onClick={() => setSearchInput('')}
                aria-label={t('toolbar.clearInput')}
                title={t('toolbar.clearInput')}
                className="reticle-target absolute top-1/2 right-2 -translate-y-1/2 text-[var(--text-muted)] hover:text-[var(--text-primary)]"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
          </div>

          {/* 模块选择下拉 */}
          <div className="relative">
            <select
              id="oplog-module-filter"
              value={moduleFilter}
              onChange={(event) => {
                const value = event.target.value
                if (isMemberOf(MODULE_FILTERS, value)) {
                  setModuleFilter(value)
                  setPage(1)
                }
              }}
              aria-label={t('toolbar.filterModule')}
              className="h-8 appearance-none rounded-lg border border-[var(--border-strong)] bg-[var(--bg-surface-solid)] pr-7 pl-2.5 text-xs text-[var(--text-primary)] transition-colors outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--ring)]"
            >
              {MODULE_FILTERS.map((module) => (
                <option key={module} value={module}>
                  {t(`modules.${module}`)}
                </option>
              ))}
            </select>
            <Filter className="pointer-events-none absolute top-1/2 right-2.5 h-3 w-3 -translate-y-1/2 text-[var(--text-muted)]" />
          </div>

          {/* 秒级精细时间范围选择器 */}
          <DateTimeRangePicker
            value={timeRange}
            onChange={(val) => {
              setTimeRange(val)
              setPage(1)
            }}
            t={tAlarm}
          />

          {/* 状态码快速筛选分段器 */}
          <div
            role="group"
            aria-label={t('toolbar.filterStatus')}
            className="hidden items-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] p-0.5 text-xs sm:flex"
          >
            {STATUS_SEGMENTS.map((segment) => (
              <button
                key={segment.id}
                type="button"
                aria-pressed={statusFilter === segment.id}
                onClick={() => {
                  setStatusFilter(segment.id)
                  setPage(1)
                }}
                className={`rounded-md px-2.5 py-1 font-medium transition-colors ${
                  statusFilter === segment.id
                    ? getToneClasses(segment.tone).solid
                    : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                }`}
              >
                {t(segment.labelKey)}
              </button>
            ))}
          </div>

          {/* 重置筛选 */}
          {isFiltered && (
            <button
              type="button"
              onClick={handleResetFilters}
              title={t('toolbar.resetFilters')}
              className="reticle-target flex h-8 items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] px-2 text-xs text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)]"
            >
              <RotateCcw className="h-3 w-3" />
              <span className="hidden md:inline">{t('toolbar.resetFilters')}</span>
            </button>
          )}
        </div>

        {/* 右侧工具集：密度切换、手动刷新 */}
        <OplogToolbar
          isLoading={isLoading}
          onRefresh={refresh}
          density={density}
          onDensityChange={setDensity}
          pageCount={logs.length}
        />
      </section>

      {/* 错误提示横幅 */}
      {error && (
        <div
          className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-[var(--status-danger)]/30 bg-[var(--status-danger)]/8 px-3.5 py-2.5 text-xs text-[var(--status-danger)]"
          role="alert"
        >
          <span>{t('loadError', { error: error || t('unknownError') })}</span>
          <button
            type="button"
            onClick={refresh}
            className="reticle-target inline-flex items-center gap-1 rounded-md px-2 py-1 font-medium transition-colors hover:bg-[var(--status-danger)]/10"
          >
            <RefreshCw className="h-3 w-3" />
            {t('retry')}
          </button>
        </div>
      )}

      {/* 数据主表格 */}
      <section className="frosted-glass min-h-0 flex-1 overflow-hidden rounded-2xl shadow-xs">
        <div className="h-full overflow-auto" ref={tableContainerRef}>
          <table className="w-full min-w-[1000px] text-left text-xs">
            <thead className="sticky top-0 z-10 border-b border-[var(--border)] bg-[var(--bg-secondary)]/90 tracking-wider text-[var(--text-muted)] uppercase backdrop-blur-md">
              <tr>
                <th className="w-24 px-4 py-3 font-medium">{t('columns.method')}</th>
                <th className="w-28 px-4 py-3 font-medium">{t('columns.status')}</th>
                <th className="px-4 py-3 font-medium">{t('columns.path')}</th>
                <th className="w-28 px-4 py-3 font-medium">{t('columns.module')}</th>
                <th className="w-28 px-4 py-3 font-medium">{t('columns.action')}</th>
                <th className="w-28 px-4 py-3 font-medium">{t('columns.username')}</th>
                <th className="w-32 px-4 py-3 font-medium">{t('columns.ip')}</th>
                <th className="w-28 px-4 py-3 font-medium">{t('columns.duration')}</th>
                <th className="w-44 px-4 py-3 font-medium">{t('columns.time')}</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[var(--border)] text-[var(--text-secondary)]">
              {isLoading ? (
                <LogTableSkeleton columnsCount={9} rowsCount={8} compact={density === 'compact'} />
              ) : logs.length === 0 ? (
                <tr>
                  <td colSpan={9} className="p-0">
                    <LogEmptyState
                      isFiltered={isFiltered}
                      onClearFilters={isFiltered ? handleResetFilters : undefined}
                    />
                  </td>
                </tr>
              ) : (
                logs.map((log) => {
                  const methodClasses = getToneClasses(methodTone(log.method))
                  const statusClasses = getToneClasses(statusTone(log.statusCode))

                  return (
                    <tr
                      key={log.id}
                      onClick={() => setInspectingLog({ kind: 'operation', data: log })}
                      className="group cursor-pointer transition-colors hover:bg-[var(--accent-soft)]/45 focus-visible:bg-[var(--accent-soft)]/45 focus-visible:outline-none"
                      tabIndex={0}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault()
                          setInspectingLog({ kind: 'operation', data: log })
                        }
                      }}
                    >
                      {/* Method 标签 */}
                      <td className={`px-4 ${rowPaddingClass} whitespace-nowrap`}>
                        <span
                          className={`inline-block w-16 rounded border py-0.5 text-center font-mono text-[10px] font-bold tracking-wider uppercase ${methodClasses.badge}`}
                        >
                          {log.method}
                        </span>
                      </td>

                      {/* Status Code 药丸 */}
                      <td className={`px-4 ${rowPaddingClass} whitespace-nowrap`}>
                        <span
                          className={`inline-flex items-center gap-1.5 rounded-md border px-2 py-0.5 font-mono text-[11px] font-semibold ${statusClasses.badge}`}
                        >
                          <span className={`h-1.5 w-1.5 rounded-full ${statusClasses.dot}`} />
                          {t(`status.${classifyHttpStatus(log.statusCode)}`, {
                            code: log.statusCode,
                          })}
                        </span>
                      </td>

                      {/* Path 路径 */}
                      <td
                        className={`font-data max-w-[320px] px-4 ${rowPaddingClass}`}
                        title={log.path}
                      >
                        <span className="block truncate font-medium text-[var(--text-primary)]">
                          {log.path}
                        </span>
                      </td>

                      {/* Module */}
                      <td className={`px-4 ${rowPaddingClass} whitespace-nowrap`}>
                        <span className="font-data inline-flex items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--bg-surface-solid)] px-2 py-0.5 text-[11px] text-[var(--text-primary)]">
                          <Layers className="h-3 w-3 text-[var(--text-muted)]" />
                          {log.module}
                        </span>
                      </td>

                      {/* Action */}
                      <td
                        className={`px-4 ${rowPaddingClass} font-medium whitespace-nowrap text-[var(--text-primary)]`}
                      >
                        {log.action}
                      </td>

                      {/* Username */}
                      <td
                        className={`px-4 ${rowPaddingClass} whitespace-nowrap text-[var(--text-primary)]`}
                      >
                        {log.username || (
                          <span className="text-[var(--text-muted)]">{t('unknown')}</span>
                        )}
                      </td>

                      {/* IP */}
                      <td
                        className={`font-data px-4 ${rowPaddingClass} whitespace-nowrap text-[var(--text-muted)]`}
                      >
                        {log.ip || '-'}
                      </td>

                      {/* Duration */}
                      <td
                        className={`font-data px-4 ${rowPaddingClass} whitespace-nowrap tabular-nums`}
                      >
                        <span className={getToneClasses(latencyTone(log.durationMs)).text}>
                          {t('durationValue', { value: log.durationMs })}
                        </span>
                      </td>

                      {/* Time */}
                      <td
                        className={`font-data px-4 ${rowPaddingClass} whitespace-nowrap text-[var(--text-muted)] tabular-nums`}
                      >
                        <div className="flex items-center justify-between gap-2">
                          <span>{formatTimestamp(log.createdAt, i18n.language)}</span>
                          <span className="rounded px-1.5 py-0.5 text-[10px] text-[var(--accent)] opacity-0 transition-opacity group-hover:opacity-100">
                            {t('toolbar.viewDetails')}
                          </span>
                        </div>
                      </td>
                    </tr>
                  )
                })
              )}
            </tbody>
          </table>
        </div>
      </section>

      {/* 现代 SaaS 标准底部分页栏 */}
      <LogPaginationBar
        page={page}
        pageSize={pageSize}
        onPageChange={handlePageChange}
        onPageSizeChange={handlePageSizeChange}
        hasMore={hasMore}
        isLoading={isLoading}
        totalOnCurrentPage={logs.length}
      />

      {/* 侧滑详情检查器抽屉 */}
      <LogDetailDrawer log={inspectingLog} onClose={() => setInspectingLog(null)} />
    </div>
  )
}
