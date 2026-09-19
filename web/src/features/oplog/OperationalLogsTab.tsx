import { useEffect, useMemo, useRef, useState, type ReactElement } from 'react'
import { Camera, Filter, Layers, RefreshCw, RotateCcw, Search, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import {
  DateTimeRangePicker,
  type DateTimeRangeValue,
} from '@/features/alarms/components/DateTimeRangePicker'
import { resolveEffectiveTimeRange } from '@/features/alarms/utils'
import { useDebounce } from '@/hooks/use-debounce'
import { formatTimestamp } from '@/lib/time'
import { LogDetailDrawer, type InspectableLog } from './components/LogDetailDrawer'
import { LogEmptyState } from './components/LogEmptyState'
import { LogPaginationBar } from './components/LogPaginationBar'
import { LogTableSkeleton } from './components/LogTableSkeleton'
import { OperationalStatsCards } from './components/OperationalStatsCards'
import { OplogToolbar, type LogDensity } from './components/OplogToolbar'
import { useOperationalLogs } from './hooks/useOperationalLogs'
import {
  EVENT_FILTERS,
  isMemberOf,
  type EventFilter,
  type LevelFilter,
  type TargetFilter,
  TARGET_FILTERS,
} from './logFilters'
import { DEFAULT_LOG_PAGE_SIZE } from './logPaging'
import { getToneClasses, LEVEL_ICONS, levelTone, type LogTone } from './logTone'

/** 级别分段筛选；tone 只用于选中态的实心配色 */
const LEVEL_SEGMENTS: readonly { id: LevelFilter; tone: LogTone }[] = [
  { id: 'all', tone: 'accent' },
  { id: 'info', tone: 'accent' },
  { id: 'warn', tone: 'warning' },
  { id: 'error', tone: 'danger' },
]

export function OperationalLogsTab(): ReactElement {
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
  const [level, setLevel] = useState<LevelFilter>('all')
  const [target, setTarget] = useState<TargetFilter>('all')
  const [event, setEvent] = useState<EventFilter>('all')
  const [cameraIdInput, setCameraIdInput] = useState('')
  const debouncedCameraId = useDebounce(cameraIdInput.trim(), 250)

  // 摄像头筛选变动重置回第 1 页
  useEffect(() => {
    setPage(1)
  }, [debouncedCameraId])

  // 交互控制
  const [density, setDensity] = useState<LogDensity>('comfortable')
  const [inspectingLog, setInspectingLog] = useState<InspectableLog | null>(null)

  const { logs, isLoading, hasMore, error, refresh } = useOperationalLogs(
    {
      level,
      target,
      event,
      cameraId: debouncedCameraId,
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
    level !== 'all' ||
    target !== 'all' ||
    event !== 'all' ||
    timeRange.quickPreset !== 'all' ||
    cameraIdInput.trim().length > 0

  function handleResetFilters(): void {
    setLevel('all')
    setTarget('all')
    setEvent('all')
    setTimeRange({ quickPreset: 'all' })
    setCameraIdInput('')
    setPage(1)
  }

  const rowPaddingClass = density === 'compact' ? 'py-2' : 'py-3'

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col gap-3">
      {/* 顶部统计指标卡片（口径为当前页，可点击联动 level 筛选） */}
      <OperationalStatsCards
        logs={logs}
        levelFilter={level}
        onSelectLevelFilter={(filter) => {
          setLevel(filter)
          setPage(1)
        }}
      />

      {/* 现代智能控制栏 */}
      <section className="frosted-glass flex flex-wrap items-center justify-between gap-2.5 rounded-xl p-2.5">
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          {/* 事件标记精确检索（服务端 event 参数） */}
          <div className="relative min-w-44 flex-1 sm:max-w-xs">
            <Search className="absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)]" />
            <select
              id="operational-event-filter"
              value={event}
              onChange={(e) => {
                const value = e.target.value
                if (isMemberOf(EVENT_FILTERS, value)) {
                  setEvent(value)
                  setPage(1)
                }
              }}
              aria-label={t('operational.filterEvent')}
              className="h-8 w-full appearance-none rounded-lg border border-[var(--border-strong)] bg-[var(--bg-surface-solid)] pr-7 pl-8 text-xs text-[var(--text-primary)] transition-colors outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--ring)]"
            >
              {EVENT_FILTERS.map((tag) => (
                <option key={tag} value={tag}>
                  {tag === 'all' ? t('operational.events.all') : tag}
                </option>
              ))}
            </select>
            <Filter className="pointer-events-none absolute top-1/2 right-2.5 h-3 w-3 -translate-y-1/2 text-[var(--text-muted)]" />
          </div>

          {/* 级别快速切换分段器 */}
          <div
            role="group"
            aria-label={t('operational.filterLevel')}
            className="flex items-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] p-0.5 text-xs"
          >
            {LEVEL_SEGMENTS.map((segment) => (
              <button
                key={segment.id}
                type="button"
                aria-pressed={level === segment.id}
                onClick={() => {
                  setLevel(segment.id)
                  setPage(1)
                }}
                className={`rounded-md px-2.5 py-1 font-medium transition-colors ${
                  level === segment.id
                    ? getToneClasses(segment.tone).solid
                    : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                }`}
              >
                {t(`operational.levels.${segment.id}`)}
              </button>
            ))}
          </div>

          {/* 目标模块架构层 Target 下拉 */}
          <div className="relative">
            <select
              id="operational-target-filter"
              value={target}
              onChange={(e) => {
                const value = e.target.value
                if (isMemberOf(TARGET_FILTERS, value)) {
                  setTarget(value)
                  setPage(1)
                }
              }}
              aria-label={t('operational.filterTarget')}
              className="h-8 appearance-none rounded-lg border border-[var(--border-strong)] bg-[var(--bg-surface-solid)] pr-7 pl-2.5 text-xs text-[var(--text-primary)] transition-colors outline-none focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--ring)]"
            >
              {TARGET_FILTERS.map((opt) => (
                <option key={opt} value={opt}>
                  {t(`operational.targets.${opt}`)}
                </option>
              ))}
            </select>
            <Layers className="pointer-events-none absolute top-1/2 right-2.5 h-3 w-3 -translate-y-1/2 text-[var(--text-muted)]" />
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

          {/* 摄像头 ID 过滤 */}
          <div className="relative w-36">
            <input
              id="operational-camera-filter"
              type="text"
              value={cameraIdInput}
              onChange={(e) => setCameraIdInput(e.target.value)}
              placeholder={t('operational.cameraPlaceholder')}
              aria-label={t('operational.filterCamera')}
              className="h-8 w-full rounded-lg border border-[var(--border-strong)] bg-[var(--bg-surface-solid)] pr-7 pl-2.5 text-xs text-[var(--text-primary)] transition-colors outline-none placeholder:text-[var(--text-muted)] focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--ring)]"
            />
            {cameraIdInput && (
              <button
                type="button"
                onClick={() => setCameraIdInput('')}
                aria-label={t('toolbar.clearInput')}
                title={t('toolbar.clearInput')}
                className="reticle-target absolute top-1/2 right-2 -translate-y-1/2 text-[var(--text-muted)] hover:text-[var(--text-primary)]"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
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
          className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-[var(--destructive)]/30 bg-[var(--destructive)]/8 px-3.5 py-2.5 text-xs text-[var(--destructive)]"
          role="alert"
        >
          <span>{t('operational.loadError', { error: error || t('unknownError') })}</span>
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

      {/* 数据主表格 */}
      <section className="frosted-glass min-h-0 flex-1 overflow-hidden rounded-2xl shadow-xs">
        <div className="h-full overflow-auto" ref={tableContainerRef}>
          <table className="w-full min-w-[960px] text-left text-xs">
            <thead className="sticky top-0 z-10 border-b border-[var(--border)] bg-[var(--bg-secondary)]/90 tracking-wider text-[var(--text-muted)] uppercase backdrop-blur-md">
              <tr>
                <th className="w-24 px-4 py-3 font-medium">{t('operational.columns.level')}</th>
                <th className="w-28 px-4 py-3 font-medium">{t('operational.columns.target')}</th>
                <th className="w-40 px-4 py-3 font-medium">{t('operational.columns.event')}</th>
                <th className="w-32 px-4 py-3 font-medium">{t('operational.columns.camera')}</th>
                <th className="px-4 py-3 font-medium">{t('operational.columns.message')}</th>
                <th className="w-44 px-4 py-3 font-medium">{t('operational.columns.time')}</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[var(--border)] text-[var(--text-secondary)]">
              {isLoading ? (
                <LogTableSkeleton columnsCount={6} rowsCount={8} compact={density === 'compact'} />
              ) : logs.length === 0 ? (
                <tr>
                  <td colSpan={6} className="p-0">
                    <LogEmptyState
                      isFiltered={isFiltered}
                      onClearFilters={isFiltered ? handleResetFilters : undefined}
                    />
                  </td>
                </tr>
              ) : (
                logs.map((log) => {
                  const LevelIcon = LEVEL_ICONS[log.level]
                  const levelClasses = getToneClasses(levelTone(log.level))
                  const hasExtra = Boolean(log.extraJson?.trim())

                  return (
                    <tr
                      key={log.id}
                      onClick={() => setInspectingLog({ kind: 'operational', data: log })}
                      className="group cursor-pointer transition-colors hover:bg-[var(--accent-soft)]/45 focus-visible:bg-[var(--accent-soft)]/45 focus-visible:outline-none"
                      tabIndex={0}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault()
                          setInspectingLog({ kind: 'operational', data: log })
                        }
                      }}
                    >
                      {/* Level 徽章 */}
                      <td className={`px-4 ${rowPaddingClass} whitespace-nowrap`}>
                        <span
                          className={`inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-[10px] font-bold tracking-wider ${levelClasses.badge}`}
                        >
                          <LevelIcon className="h-3 w-3" />
                          {t(`operational.levels.${log.level}`)}
                        </span>
                      </td>

                      {/* Target 模块架构层 */}
                      <td
                        className={`px-4 ${rowPaddingClass} whitespace-nowrap text-[var(--text-primary)]`}
                      >
                        <span className="font-data inline-flex items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--bg-surface-solid)] px-2 py-0.5 text-[11px] text-[var(--text-primary)]">
                          <Layers className="h-3 w-3 text-[var(--text-muted)]" />
                          {t(`operational.targets.${log.target}`, { defaultValue: log.target })}
                        </span>
                      </td>

                      {/* Event 事件名 */}
                      <td
                        className={`font-data px-4 ${rowPaddingClass} font-semibold whitespace-nowrap text-[var(--text-primary)]`}
                      >
                        {log.event}
                      </td>

                      {/* Camera 关联摄像头 */}
                      <td className={`font-data px-4 ${rowPaddingClass} whitespace-nowrap`}>
                        {log.cameraId ? (
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation()
                              setCameraIdInput(log.cameraId ?? '')
                            }}
                            title={t('operational.filterByCamera')}
                            aria-label={t('operational.filterByCamera')}
                            className="reticle-target inline-flex items-center gap-1 rounded-md bg-[var(--accent-soft)] px-1.5 py-0.5 text-[11px] font-medium text-[var(--accent)] hover:underline"
                          >
                            <Camera className="h-3 w-3" />
                            <span>{log.cameraId}</span>
                          </button>
                        ) : (
                          <span className="text-[var(--text-muted)]">-</span>
                        )}
                      </td>

                      {/* Message 详情与元数据标识 */}
                      <td className={`px-4 ${rowPaddingClass}`}>
                        <div className="flex items-center gap-2">
                          <span className="leading-relaxed font-medium text-[var(--text-primary)]">
                            {log.message}
                          </span>
                          {hasExtra && (
                            <span
                              className="inline-flex shrink-0 items-center rounded border border-[var(--accent)]/30 bg-[var(--accent-soft)] px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[var(--accent)]"
                              title={t('operational.hasPayload')}
                            >
                              JSON
                            </span>
                          )}
                        </div>
                      </td>

                      {/* Time 时间 */}
                      <td
                        className={`font-data px-4 ${rowPaddingClass} whitespace-nowrap text-[var(--text-muted)] tabular-nums`}
                      >
                        <div className="flex items-center justify-between gap-2">
                          <span>{formatTimestamp(log.tsMs, i18n.language)}</span>
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
