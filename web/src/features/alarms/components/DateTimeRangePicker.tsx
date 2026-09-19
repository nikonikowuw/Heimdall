import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  CalendarClock,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Clock,
  RotateCcw,
  Sparkles,
  X,
} from 'lucide-react'
import { formatTimestamp } from '@/lib/time'

export type QuickTimePreset =
  'all' | '5m' | '15m' | '30m' | '1h' | '24h' | 'today' | '7d' | 'custom'

export interface DateTimeRangeValue {
  startTime?: number // 13位 UTC 毫秒时间戳
  endTime?: number // 13位 UTC 毫秒时间戳
  quickPreset: QuickTimePreset
}

export interface DateTimeRangePickerProps {
  value: DateTimeRangeValue
  onChange: (val: DateTimeRangeValue) => void
  t: (key: string, options?: Record<string, unknown>) => string
}

interface CalendarDay {
  year: number
  month: number
  day: number
  isCurrentMonth: boolean
  date: Date
}

function pad(n: number): string {
  return String(n).padStart(2, '0')
}

function parseTimePart(val: string, fallback: number): number {
  const n = parseInt(val, 10)
  return Number.isNaN(n) ? fallback : n
}

function clampPad(val: string, max: number): string {
  const num = Math.min(max, Math.max(0, parseInt(val, 10) || 0))
  return pad(num)
}

function wrapStep(val: string, delta: number, max: number): string {
  const current = parseInt(val, 10) || 0
  const next = (current + delta + (max + 1)) % (max + 1)
  return pad(next)
}

const PRESET_DURATIONS_MS: Partial<Record<QuickTimePreset, number>> = {
  '5m': 5 * 60_000,
  '15m': 15 * 60_000,
  '30m': 30 * 60_000,
  '1h': 3_600_000,
  '24h': 86_400_000,
  '7d': 7 * 86_400_000,
}

function calculatePresetTimestamps(preset: QuickTimePreset): {
  startTime?: number
  endTime?: number
} {
  const now = Date.now()
  if (preset === 'today') {
    const todayStart = new Date()
    todayStart.setHours(0, 0, 0, 0)
    return { startTime: todayStart.getTime(), endTime: undefined }
  }
  const duration = PRESET_DURATIONS_MS[preset]
  if (duration) {
    return { startTime: now - duration, endTime: now }
  }
  return { startTime: undefined, endTime: undefined }
}

function formatDuration(
  startMs: number,
  endMs: number,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  const diffMs = Math.max(0, endMs - startMs)
  const totalSec = Math.floor(diffMs / 1000)
  const sec = totalSec % 60
  const min = Math.floor(totalSec / 60) % 60
  const hour = Math.floor(totalSec / 3600) % 24
  const day = Math.floor(totalSec / 86400)

  const parts: string[] = []
  if (day > 0) parts.push(`${day}d`)
  if (hour > 0) parts.push(`${hour}${t('timeFilter.hourUnit')}`)
  if (min > 0) parts.push(`${min}${t('timeFilter.minuteUnit')}`)
  if (sec > 0 || parts.length === 0) parts.push(`${sec}${t('timeFilter.secondUnit')}`)
  return parts.join(' ')
}

function isSameDay(d1: Date, d2: Date): boolean {
  return (
    d1.getFullYear() === d2.getFullYear() &&
    d1.getMonth() === d2.getMonth() &&
    d1.getDate() === d2.getDate()
  )
}

function startOfDay(d: Date): Date {
  const res = new Date(d)
  res.setHours(0, 0, 0, 0)
  return res
}

function getCalendarGrid(year: number, month: number): CalendarDay[] {
  const firstDay = new Date(year, month, 1)
  const startDayOfWeek = (firstDay.getDay() + 6) % 7 // Monday = 0
  const daysInMonth = new Date(year, month + 1, 0).getDate()
  const daysInPrevMonth = new Date(year, month, 0).getDate()

  const days: CalendarDay[] = []

  for (let i = startDayOfWeek - 1; i >= 0; i--) {
    const d = daysInPrevMonth - i
    const m = month === 0 ? 11 : month - 1
    const y = month === 0 ? year - 1 : year
    days.push({ year: y, month: m, day: d, isCurrentMonth: false, date: new Date(y, m, d) })
  }

  for (let d = 1; d <= daysInMonth; d++) {
    days.push({ year, month, day: d, isCurrentMonth: true, date: new Date(year, month, d) })
  }

  const remaining = 42 - days.length
  for (let d = 1; d <= remaining; d++) {
    const m = month === 11 ? 0 : month + 1
    const y = month === 11 ? year + 1 : year
    days.push({ year: y, month: m, day: d, isCurrentMonth: false, date: new Date(y, m, d) })
  }

  return days
}

export function DateTimeRangePicker({
  value,
  onChange,
  t,
}: DateTimeRangePickerProps): React.ReactElement {
  const [isOpen, setIsOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)

  const [popoverStyle, setPopoverStyle] = useState<React.CSSProperties>({})

  // 日期范围（年月日）：支持单月日历连续连线范围选定
  const [rangeStart, setRangeStart] = useState<Date>(() =>
    value.startTime ? new Date(value.startTime) : new Date(Date.now() - 3600 * 1000),
  )
  const [rangeEnd, setRangeEnd] = useState<Date | null>(() =>
    value.endTime ? new Date(value.endTime) : new Date(),
  )
  const [hoverDate, setHoverDate] = useState<Date | null>(null)

  // 时间微调（时分秒字符串：HH, mm, ss）
  const [startHour, setStartHour] = useState('00')
  const [startMin, setStartMin] = useState('00')
  const [startSec, setStartSec] = useState('00')

  const [endHour, setEndHour] = useState('23')
  const [endMin, setEndMin] = useState('59')
  const [endSec, setEndSec] = useState('59')

  // 当前草稿预设
  const [draftPreset, setDraftPreset] = useState<QuickTimePreset>(value.quickPreset)

  // 日历展示月份与年份
  const [viewYear, setViewYear] = useState<number>(() =>
    (value.startTime ? new Date(value.startTime) : new Date()).getFullYear(),
  )
  const [viewMonth, setViewMonth] = useState<number>(() =>
    (value.startTime ? new Date(value.startTime) : new Date()).getMonth(),
  )

  // 打开弹窗时将外部状态同步到草稿
  useEffect(() => {
    if (!isOpen) return

    const s = value.startTime ? new Date(value.startTime) : new Date(Date.now() - 3600 * 1000)
    const e = value.endTime ? new Date(value.endTime) : new Date()

    setRangeStart(s)
    setRangeEnd(e)
    setHoverDate(null)
    setDraftPreset(value.quickPreset)

    setStartHour(pad(s.getHours()))
    setStartMin(pad(s.getMinutes()))
    setStartSec(pad(s.getSeconds()))

    setEndHour(pad(e.getHours()))
    setEndMin(pad(e.getMinutes()))
    setEndSec(pad(e.getSeconds()))

    setViewYear(s.getFullYear())
    setViewMonth(s.getMonth())
  }, [isOpen, value])

  // 计算并更新浮层视口坐标（自适应防溢出与上下翻转）
  const updatePosition = useCallback(() => {
    if (!triggerRef.current) return
    const rect = triggerRef.current.getBoundingClientRect()
    const popoverWidth = 590
    const popoverHeight = 440
    const margin = 8

    const spaceBelow = window.innerHeight - rect.bottom
    const spaceAbove = rect.top
    const openUpwards = spaceBelow < popoverHeight && spaceAbove > spaceBelow

    let left = rect.left
    if (left + popoverWidth > window.innerWidth - 16) {
      left = Math.max(16, rect.right - popoverWidth)
    }

    setPopoverStyle({
      position: 'fixed',
      left: `${left}px`,
      ...(openUpwards
        ? { bottom: `${window.innerHeight - rect.top + margin}px` }
        : { top: `${rect.bottom + margin}px` }),
      zIndex: 9999,
    })
  }, [])

  useEffect(() => {
    if (!isOpen) return

    updatePosition()

    const handleScroll = (e: Event): void => {
      if (dropdownRef.current && dropdownRef.current.contains(e.target as Node)) {
        return
      }
      updatePosition()
    }

    const handleResize = (): void => updatePosition()

    const handleClickOutside = (e: MouseEvent): void => {
      const target = e.target as Node
      if (
        triggerRef.current &&
        !triggerRef.current.contains(target) &&
        dropdownRef.current &&
        !dropdownRef.current.contains(target)
      ) {
        setIsOpen(false)
      }
    }

    const handleKeyDown = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        setIsOpen(false)
      }
    }

    window.addEventListener('scroll', handleScroll, true)
    window.addEventListener('resize', handleResize)
    document.addEventListener('mousedown', handleClickOutside)
    document.addEventListener('keydown', handleKeyDown)

    return () => {
      window.removeEventListener('scroll', handleScroll, true)
      window.removeEventListener('resize', handleResize)
      document.removeEventListener('mousedown', handleClickOutside)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [isOpen, updatePosition])

  // 合成完整的 draft 起始与截止毫秒时间戳
  const currentStartMs = useMemo(() => {
    if (draftPreset === 'all') return undefined
    const d = new Date(rangeStart)
    d.setHours(
      parseTimePart(startHour, 0),
      parseTimePart(startMin, 0),
      parseTimePart(startSec, 0),
      0,
    )
    return d.getTime()
  }, [draftPreset, rangeStart, startHour, startMin, startSec])

  const currentEndMs = useMemo(() => {
    if (draftPreset === 'all') return undefined
    const d = new Date(rangeEnd || rangeStart)
    d.setHours(
      parseTimePart(endHour, 23),
      parseTimePart(endMin, 59),
      parseTimePart(endSec, 59),
      999,
    )
    return d.getTime()
  }, [draftPreset, rangeEnd, rangeStart, endHour, endMin, endSec])

  const isInvalid = useMemo(() => {
    if (draftPreset === 'all') return false
    if (!currentStartMs || !currentEndMs) return false
    return currentStartMs > currentEndMs
  }, [draftPreset, currentStartMs, currentEndMs])

  // 快捷预设列表
  const presets = useMemo<Array<{ key: QuickTimePreset; label: string }>>(
    () => [
      { key: 'all', label: t('timeFilter.reset') },
      { key: '5m', label: t('timeFilter.past5Minutes') },
      { key: '15m', label: t('timeFilter.past15Minutes') },
      { key: '30m', label: t('timeFilter.past30Minutes') },
      { key: '1h', label: t('filter.past1Hour') },
      { key: '24h', label: t('filter.past24Hours') },
      { key: 'today', label: t('timeFilter.today') },
      { key: '7d', label: t('filter.past7Days') },
    ],
    [t],
  )

  const handleSelectPreset = (preset: QuickTimePreset): void => {
    setDraftPreset(preset)
    if (preset === 'all') {
      onChange({ quickPreset: 'all', startTime: undefined, endTime: undefined })
      setIsOpen(false)
      return
    }
    if (preset === 'today') {
      const todayStart = new Date()
      todayStart.setHours(0, 0, 0, 0)
      onChange({
        quickPreset: 'today',
        startTime: todayStart.getTime(),
        endTime: undefined,
      })
      setIsOpen(false)
      return
    }
    const timestamps = calculatePresetTimestamps(preset)
    if (timestamps.startTime !== undefined) {
      onChange({
        quickPreset: preset,
        startTime: timestamps.startTime,
        endTime: timestamps.endTime,
      })
      setIsOpen(false)
    }
  }

  const handleDayClick = (date: Date): void => {
    setDraftPreset('custom')
    if (!rangeEnd) {
      if (date.getTime() < rangeStart.getTime()) {
        setRangeStart(date)
        setRangeEnd(rangeStart)
      } else {
        setRangeEnd(date)
      }
    } else {
      setRangeStart(date)
      setRangeEnd(null)
    }
    setHoverDate(null)
  }

  const handleSetNow = (target: 'start' | 'end'): void => {
    setDraftPreset('custom')
    const now = new Date()
    const h = pad(now.getHours())
    const m = pad(now.getMinutes())
    const s = pad(now.getSeconds())
    if (target === 'start') {
      setRangeStart(now)
      setStartHour(h)
      setStartMin(m)
      setStartSec(s)
    } else {
      setRangeEnd(now)
      setEndHour(h)
      setEndMin(m)
      setEndSec(s)
    }
  }

  const handleSetBoundary = (target: 'start' | 'end', type: 'startOfDay' | 'endOfDay'): void => {
    setDraftPreset('custom')
    const [h, m, s] = type === 'startOfDay' ? ['00', '00', '00'] : ['23', '59', '59']
    if (target === 'start') {
      setStartHour(h)
      setStartMin(m)
      setStartSec(s)
    } else {
      setEndHour(h)
      setEndMin(m)
      setEndSec(s)
    }
  }

  const handlePrevMonth = (): void => {
    if (viewMonth === 0) {
      setViewYear((y) => y - 1)
      setViewMonth(11)
    } else {
      setViewMonth((m) => m - 1)
    }
  }

  const handleNextMonth = (): void => {
    if (viewMonth === 11) {
      setViewYear((y) => y + 1)
      setViewMonth(0)
    } else {
      setViewMonth((m) => m + 1)
    }
  }

  const handleApply = (): void => {
    if (isInvalid) return
    if (draftPreset === 'all') {
      onChange({ quickPreset: 'all', startTime: undefined, endTime: undefined })
    } else {
      onChange({
        quickPreset: 'custom',
        startTime: currentStartMs,
        endTime: currentEndMs,
      })
    }
    setIsOpen(false)
  }

  const triggerLabel = useMemo(() => {
    if (value.quickPreset === 'all' || (!value.startTime && !value.endTime)) {
      return t('timeFilter.reset')
    }
    if (value.quickPreset !== 'custom') {
      const found = presets.find((p) => p.key === value.quickPreset)
      if (found) return found.label
    }
    if (value.startTime && value.endTime) {
      const s = formatTimestamp(value.startTime)
      const e = formatTimestamp(value.endTime)
      return `${s} ~ ${e}`
    }
    if (value.startTime) {
      return `≥ ${formatTimestamp(value.startTime)}`
    }
    if (value.endTime) {
      return `≤ ${formatTimestamp(value.endTime)}`
    }
    return t('timeFilter.reset')
  }, [value, presets, t])

  const hasActiveFilter = value.quickPreset !== 'all' && (value.startTime || value.endTime)

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => {
          if (!isOpen) updatePosition()
          setIsOpen((prev) => !prev)
        }}
        className={`group flex items-center gap-2 rounded-xl border px-3 py-1.5 text-xs font-medium transition-all ${
          hasActiveFilter
            ? 'border-[var(--accent)] bg-[var(--accent-soft)] text-[var(--accent)] shadow-xs'
            : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--text-primary)]'
        }`}
        title={t('timeFilter.customRange')}
      >
        <CalendarClock className="h-3.5 w-3.5 shrink-0" />
        <span className="max-w-[280px] truncate font-mono text-[11px]">{triggerLabel}</span>
        {hasActiveFilter ? (
          <span
            onClick={(e) => {
              e.stopPropagation()
              onChange({ quickPreset: 'all', startTime: undefined, endTime: undefined })
              setIsOpen(false)
            }}
            className="ml-0.5 rounded-md p-0.5 text-[var(--text-muted)] hover:bg-rose-500/20 hover:text-rose-400"
            title={t('timeFilter.clear')}
          >
            <X className="h-3 w-3" />
          </span>
        ) : (
          <ChevronDown
            className={`h-3 w-3 shrink-0 opacity-60 transition-transform duration-200 ${
              isOpen ? 'rotate-180' : ''
            }`}
          />
        )}
      </button>

      {isOpen &&
        typeof document !== 'undefined' &&
        createPortal(
          <div
            ref={dropdownRef}
            style={popoverStyle}
            className="animate-in fade-in-0 zoom-in-95 flex w-[600px] max-w-[calc(100vw-32px)] overflow-hidden rounded-2xl border border-[var(--border-strong)] bg-[var(--bg-surface)]/95 shadow-2xl backdrop-blur-2xl duration-150"
          >
            {/* 左侧一栏：高频快捷区间 */}
            <div className="flex w-[140px] shrink-0 flex-col justify-between border-r border-[var(--border)] bg-[var(--bg-secondary)]/40 p-3">
              <div>
                <div className="mb-2 text-[10px] font-bold tracking-wider text-[var(--text-muted)] uppercase">
                  {t('timeFilter.quickPresets')}
                </div>
                <div className="space-y-1">
                  {presets.map((p) => {
                    const isActive = draftPreset === p.key
                    return (
                      <button
                        key={p.key}
                        type="button"
                        onClick={() => handleSelectPreset(p.key)}
                        className={`flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-xs font-medium transition-all ${
                          isActive
                            ? 'bg-[var(--accent)] font-semibold text-white shadow-xs'
                            : 'text-[var(--text-secondary)] hover:bg-[var(--bg-surface)] hover:text-[var(--text-primary)]'
                        }`}
                      >
                        <span>{p.label}</span>
                        {isActive && <Check className="h-3 w-3" />}
                      </button>
                    )
                  })}
                </div>
              </div>

              <button
                type="button"
                onClick={() => handleSelectPreset('all')}
                className="mt-4 flex w-full items-center justify-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-1.5 text-xs text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
              >
                <RotateCcw className="h-3 w-3" />
                <span>{t('timeFilter.reset')}</span>
              </button>
            </div>

            {/* 右侧一栏：连续范围日历 + 段落时间微调 */}
            <div className="flex flex-1 flex-col justify-between p-4">
              <div>
                <div className="mb-2 flex items-center justify-between">
                  <div className="font-mono text-xs font-bold text-[var(--text-primary)]">
                    {viewYear} / {pad(viewMonth + 1)}
                  </div>

                  <div className="flex items-center gap-1">
                    <button
                      type="button"
                      onClick={handlePrevMonth}
                      className="rounded-lg p-1 text-[var(--text-muted)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
                    >
                      <ChevronLeft className="h-4 w-4" />
                    </button>
                    <button
                      type="button"
                      onClick={handleNextMonth}
                      className="rounded-lg p-1 text-[var(--text-muted)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
                    >
                      <ChevronRight className="h-4 w-4" />
                    </button>
                  </div>
                </div>

                <ContinuousCalendar
                  year={viewYear}
                  month={viewMonth}
                  rangeStart={rangeStart}
                  rangeEnd={rangeEnd}
                  hoverDate={hoverDate}
                  onDayClick={handleDayClick}
                  onDayHover={(d) => setHoverDate(d)}
                />

                <div className="my-3 border-t border-[var(--border)]/60" />

                <div className="space-y-2">
                  <div className="flex items-center justify-between gap-2 text-xs">
                    <span className="w-16 font-medium text-[var(--text-muted)]">
                      {t('timeFilter.start')}
                    </span>
                    <SegmentedTimeInput
                      hour={startHour}
                      minute={startMin}
                      second={startSec}
                      onChange={(h, m, s) => {
                        setDraftPreset('custom')
                        setStartHour(h)
                        setStartMin(m)
                        setStartSec(s)
                      }}
                    />
                    <div className="flex items-center gap-1">
                      <button
                        type="button"
                        onClick={() => handleSetBoundary('start', 'startOfDay')}
                        className="rounded-md border border-[var(--border)] bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)] hover:border-[var(--accent)]"
                        title={t('timeFilter.startOfDay')}
                      >
                        00:00
                      </button>
                      <button
                        type="button"
                        onClick={() => handleSetNow('start')}
                        className="rounded-md border border-[var(--border)] bg-[var(--bg-surface)] px-1.5 py-0.5 text-[10px] text-[var(--text-secondary)] hover:border-[var(--accent)]"
                      >
                        {t('timeFilter.now')}
                      </button>
                    </div>
                  </div>

                  <div className="flex items-center justify-between gap-2 text-xs">
                    <span className="w-16 font-medium text-[var(--text-muted)]">
                      {t('timeFilter.end')}
                    </span>
                    <SegmentedTimeInput
                      hour={endHour}
                      minute={endMin}
                      second={endSec}
                      onChange={(h, m, s) => {
                        setDraftPreset('custom')
                        setEndHour(h)
                        setEndMin(m)
                        setEndSec(s)
                      }}
                    />
                    <div className="flex items-center gap-1">
                      <button
                        type="button"
                        onClick={() => handleSetBoundary('end', 'endOfDay')}
                        className="rounded-md border border-[var(--border)] bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)] hover:border-[var(--accent)]"
                        title={t('timeFilter.endOfDay')}
                      >
                        23:59
                      </button>
                      <button
                        type="button"
                        onClick={() => handleSetNow('end')}
                        className="rounded-md border border-[var(--border)] bg-[var(--bg-surface)] px-1.5 py-0.5 text-[10px] text-[var(--text-secondary)] hover:border-[var(--accent)]"
                      >
                        {t('timeFilter.now')}
                      </button>
                    </div>
                  </div>
                </div>
              </div>

              <div className="mt-4 flex items-center justify-between border-t border-[var(--border)] pt-3">
                <div className="text-[11px]">
                  {isInvalid ? (
                    <span className="flex items-center gap-1 font-medium text-rose-500">
                      <X className="h-3.5 w-3.5" />
                      {t('timeFilter.invalidRange')}
                    </span>
                  ) : currentStartMs && currentEndMs ? (
                    <span className="flex items-center gap-1 font-mono text-[var(--text-muted)]">
                      <Sparkles className="h-3 w-3 text-[var(--accent)]" />
                      {t('timeFilter.duration', {
                        duration: formatDuration(currentStartMs, currentEndMs, t),
                      })}
                    </span>
                  ) : (
                    <span />
                  )}
                </div>

                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={() => setIsOpen(false)}
                    className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3 py-1.5 text-xs font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                  >
                    {t('common.cancel', { defaultValue: '取消' })}
                  </button>
                  <button
                    type="button"
                    onClick={handleApply}
                    disabled={isInvalid}
                    className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-1.5 text-xs font-medium text-white shadow-xs hover:opacity-90 disabled:opacity-40"
                  >
                    <Check className="h-3.5 w-3.5" />
                    <span>{t('timeFilter.apply')}</span>
                  </button>
                </div>
              </div>
            </div>
          </div>,
          document.body,
        )}
    </>
  )
}

interface ContinuousCalendarProps {
  year: number
  month: number
  rangeStart: Date
  rangeEnd: Date | null
  hoverDate: Date | null
  onDayClick: (d: Date) => void
  onDayHover: (d: Date | null) => void
}

const WEEKDAYS = ['一', '二', '三', '四', '五', '六', '日']

function getDayButtonClass(
  item: CalendarDay,
  isStart: boolean,
  isEnd: boolean,
  inRange: boolean,
): string {
  const classes = [
    'relative flex h-7 items-center justify-center font-mono text-[11px] transition-all',
  ]
  if (inRange) classes.push('bg-[var(--accent)]/15')

  if (isStart || isEnd) {
    classes.push('bg-[var(--accent)] font-bold text-white shadow-xs')
    if (isStart) classes.push('rounded-l-lg')
    if (isEnd) classes.push('rounded-r-lg')
  } else if (item.isCurrentMonth) {
    classes.push('text-[var(--text-primary)] hover:bg-[var(--accent)]/20')
  } else {
    classes.push('text-[var(--text-muted)] opacity-30 hover:opacity-60')
  }
  return classes.join(' ')
}

function ContinuousCalendar({
  year,
  month,
  rangeStart,
  rangeEnd,
  hoverDate,
  onDayClick,
  onDayHover,
}: ContinuousCalendarProps): React.ReactElement {
  const days = useMemo(() => getCalendarGrid(year, month), [year, month])
  const today = useMemo(() => new Date(), [])
  const effectiveEnd = rangeEnd || hoverDate

  return (
    <div onMouseLeave={() => onDayHover(null)}>
      <div className="mb-1 grid grid-cols-7 text-center">
        {WEEKDAYS.map((w) => (
          <div key={w} className="text-[10px] font-semibold text-[var(--text-muted)]">
            {w}
          </div>
        ))}
      </div>

      <div className="grid grid-cols-7 gap-y-1">
        {days.map((item, idx) => {
          const isTodayDate = isSameDay(item.date, today)
          const isStart = rangeStart && isSameDay(item.date, rangeStart)
          const isEnd = rangeEnd && isSameDay(item.date, rangeEnd)

          const itemTime = startOfDay(item.date).getTime()
          const startTime = rangeStart ? startOfDay(rangeStart).getTime() : 0
          const endTime = effectiveEnd ? startOfDay(effectiveEnd).getTime() : startTime

          const minTime = Math.min(startTime, endTime)
          const maxTime = Math.max(startTime, endTime)
          const inRange = Boolean(rangeStart && itemTime >= minTime && itemTime <= maxTime)

          return (
            <button
              key={`${item.year}-${item.month}-${item.day}-${idx}`}
              type="button"
              onClick={() => onDayClick(item.date)}
              onMouseEnter={() => onDayHover(item.date)}
              className={getDayButtonClass(item, Boolean(isStart), Boolean(isEnd), inRange)}
            >
              <span>{item.day}</span>
              {isTodayDate && !isStart && !isEnd && (
                <span className="absolute bottom-0.5 h-1 w-1 rounded-full bg-[var(--accent)]" />
              )}
            </button>
          )
        })}
      </div>
    </div>
  )
}

interface SegmentedTimeInputProps {
  hour: string
  minute: string
  second: string
  onChange: (h: string, m: string, s: string) => void
}

function SegmentedTimeInput({
  hour,
  minute,
  second,
  onChange,
}: SegmentedTimeInputProps): React.ReactElement {
  const hourRef = useRef<HTMLInputElement>(null)
  const minRef = useRef<HTMLInputElement>(null)
  const secRef = useRef<HTMLInputElement>(null)

  const handleHourChange = (val: string): void => {
    const clean = val.replace(/\D/g, '').slice(0, 2)
    onChange(clean, minute, second)
    if (clean.length === 2 || parseInt(clean, 10) > 2) {
      minRef.current?.focus()
      minRef.current?.select()
    }
  }

  const handleMinChange = (val: string): void => {
    const clean = val.replace(/\D/g, '').slice(0, 2)
    onChange(hour, clean, second)
    if (clean.length === 2 || parseInt(clean, 10) > 5) {
      secRef.current?.focus()
      secRef.current?.select()
    }
  }

  const handleSecChange = (val: string): void => {
    const clean = val.replace(/\D/g, '').slice(0, 2)
    onChange(hour, minute, clean)
  }

  const handleBlur = (type: 'h' | 'm' | 's'): void => {
    if (type === 'h') {
      onChange(clampPad(hour, 23), minute, second)
    } else if (type === 'm') {
      onChange(hour, clampPad(minute, 59), second)
    } else {
      onChange(hour, minute, clampPad(second, 59))
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>, type: 'h' | 'm' | 's'): void => {
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      e.preventDefault()
      const delta = e.key === 'ArrowUp' ? 1 : -1
      if (type === 'h') {
        onChange(wrapStep(hour, delta, 23), minute, second)
      } else if (type === 'm') {
        onChange(hour, wrapStep(minute, delta, 59), second)
      } else {
        onChange(hour, minute, wrapStep(second, delta, 59))
      }
    } else if (e.key === ':' || e.key === 'Enter') {
      e.preventDefault()
      if (type === 'h') minRef.current?.focus()
      if (type === 'm') secRef.current?.focus()
    }
  }

  return (
    <div className="flex items-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-0.5 font-mono text-xs text-[var(--text-primary)] shadow-2xs focus-within:border-[var(--accent)]">
      <Clock className="mr-1.5 h-3 w-3 text-[var(--text-muted)]" />
      <input
        ref={hourRef}
        type="text"
        inputMode="numeric"
        value={hour}
        onChange={(e) => handleHourChange(e.target.value)}
        onBlur={() => handleBlur('h')}
        onKeyDown={(e) => handleKeyDown(e, 'h')}
        className="w-5 bg-transparent text-center text-xs outline-none"
        title="时 (00-23)"
      />
      <span className="text-[var(--text-muted)]">:</span>
      <input
        ref={minRef}
        type="text"
        inputMode="numeric"
        value={minute}
        onChange={(e) => handleMinChange(e.target.value)}
        onBlur={() => handleBlur('m')}
        onKeyDown={(e) => handleKeyDown(e, 'm')}
        className="w-5 bg-transparent text-center text-xs outline-none"
        title="分 (00-59)"
      />
      <span className="text-[var(--text-muted)]">:</span>
      <input
        ref={secRef}
        type="text"
        inputMode="numeric"
        value={second}
        onChange={(e) => handleSecChange(e.target.value)}
        onBlur={() => handleBlur('s')}
        onKeyDown={(e) => handleKeyDown(e, 's')}
        className="w-5 bg-transparent text-center text-xs outline-none"
        title="秒 (00-59)"
      />
    </div>
  )
}
