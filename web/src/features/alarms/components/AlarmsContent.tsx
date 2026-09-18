import React from 'react'
import { AlertCircle, RotateCcw, Search } from 'lucide-react'
import type { AlarmRecord } from '../../../types'
import { AlarmCardItem } from './AlarmCardItem'
import { AlarmTableRow } from './AlarmTableRow'

export type ViewMode = 'cards' | 'table'

export interface AlarmsContentProps {
  alarms: AlarmRecord[]
  totalCount?: number
  viewMode: ViewMode
  cameraNameMap?: Record<string, string>
  selectedAlarmIds: Set<number>
  hasActiveFilters?: boolean
  searchQuery?: string
  onResetFilters?: () => void
  onClearSearch?: () => void
  onToggleSelectAlarm: (id: number, selected: boolean) => void
  onToggleSelectAll: (selected: boolean) => void
  onSelect: (alarm: AlarmRecord) => void
  onSelectCrop: (alarm: AlarmRecord) => void
  onToggleStatus: (alarm: AlarmRecord) => void
  t: (key: string, options?: Record<string, unknown>) => string
}

export function AlarmsContent({
  alarms,
  totalCount,
  viewMode,
  cameraNameMap,
  selectedAlarmIds,
  hasActiveFilters = false,
  searchQuery,
  onResetFilters,
  onClearSearch,
  onToggleSelectAlarm,
  onToggleSelectAll,
  onSelect,
  onSelectCrop,
  onToggleStatus,
  t,
}: AlarmsContentProps): React.ReactElement {
  if (alarms.length === 0) {
    if (searchQuery && searchQuery.trim()) {
      return (
        <div className="flex flex-col items-center justify-center py-20 text-center text-[var(--text-muted)]">
          <Search className="mb-3 h-10 w-10 text-[var(--text-muted)] opacity-40" />
          <p className="font-semibold text-[var(--text-secondary)]">
            {t('search.noMatch', { query: searchQuery.trim() })}
          </p>
          {onClearSearch && (
            <button
              type="button"
              onClick={onClearSearch}
              className="mt-4 flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-1.5 text-xs font-medium text-[var(--accent)] shadow-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)]/20"
            >
              <RotateCcw className="h-3.5 w-3.5" />
              <span>{t('search.clearQuery')}</span>
            </button>
          )}
        </div>
      )
    }

    return (
      <div className="flex flex-col items-center justify-center py-20 text-center text-[var(--text-muted)]">
        <AlertCircle className="mb-3 h-10 w-10 text-[var(--text-muted)] opacity-40" />
        <p className="font-semibold text-[var(--text-secondary)]">{t('empty.alarms')}</p>
        <p className="mt-1 max-w-sm text-xs opacity-75">{t('empty.alarmsDesc')}</p>
        {hasActiveFilters && onResetFilters && (
          <button
            type="button"
            onClick={onResetFilters}
            className="mt-4 flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-1.5 text-xs font-medium text-[var(--accent)] shadow-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)]/20"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            <span>{t('empty.resetFilter')}</span>
          </button>
        )}
      </div>
    )
  }

  const isAllSelected = alarms.length > 0 && alarms.every((a) => selectedAlarmIds.has(a.id))
  const isPartiallySelected = alarms.some((a) => selectedAlarmIds.has(a.id)) && !isAllSelected

  if (viewMode === 'cards') {
    return (
      <div className="space-y-3">
        <div className="flex items-center justify-between px-1 text-xs text-[var(--text-muted)]">
          <label className="flex cursor-pointer items-center gap-2 select-none">
            <input
              type="checkbox"
              checked={isAllSelected}
              ref={(el) => {
                if (el) el.indeterminate = isPartiallySelected
              }}
              onChange={(e) => onToggleSelectAll(e.target.checked)}
              className="h-4 w-4 cursor-pointer rounded border-[var(--border)] bg-[var(--bg-surface)] text-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]"
            />
            <span className="text-[11px] font-medium">{t('batch.selectAll')}</span>
          </label>
          <span className="font-mono text-[11px]">
            {totalCount !== undefined && totalCount > alarms.length
              ? `${alarms.length} / ${totalCount} ${t('columns.evidence')}`
              : `${alarms.length} ${t('columns.evidence')}`}
          </span>
        </div>

        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
          {alarms.map((alarm) => (
            <AlarmCardItem
              key={alarm.id}
              alarm={alarm}
              cameraName={cameraNameMap?.[alarm.cameraId]}
              isSelected={selectedAlarmIds.has(alarm.id)}
              onToggleSelect={onToggleSelectAlarm}
              onSelect={onSelect}
              onSelectCrop={onSelectCrop}
              onToggleStatus={onToggleStatus}
              t={t}
            />
          ))}
        </div>
      </div>
    )
  }

  return (
    <div className="w-full overflow-x-auto">
      <table className="w-full text-left text-xs text-[var(--text-secondary)]">
        <thead className="sticky top-0 z-10 border-b border-[var(--border)]/70 bg-[var(--bg-secondary)]/80 text-[11px] font-semibold tracking-wider text-[var(--text-muted)] uppercase backdrop-blur-md">
          <tr>
            <th className="w-8 px-3 py-2.5">
              <input
                type="checkbox"
                checked={isAllSelected}
                ref={(el) => {
                  if (el) el.indeterminate = isPartiallySelected
                }}
                onChange={(e) => onToggleSelectAll(e.target.checked)}
                className="h-4 w-4 cursor-pointer rounded border-[var(--border)] bg-[var(--bg-surface)] text-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]"
                aria-label={t('batch.selectAll')}
              />
            </th>
            <th className="px-3 py-2.5">{t('columns.thumbnail')}</th>
            <th className="px-3 py-2.5">{t('columns.eventId')}</th>
            <th className="px-3 py-2.5">{t('columns.camera')}</th>
            <th className="px-3 py-2.5">{t('columns.targetLabel')}</th>
            <th className="px-3 py-2.5">{t('columns.ruleType')}</th>
            <th className="px-3 py-2.5">{t('columns.severity')}</th>
            <th className="px-3 py-2.5">{t('columns.confidence')}</th>
            <th className="px-3 py-2.5">{t('columns.status')}</th>
            <th className="px-3 py-2.5">{t('columns.occurredAt')}</th>
            <th className="px-3 py-2.5 text-right">{t('columns.actions')}</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-[var(--border)]">
          {alarms.map((alarm) => (
            <AlarmTableRow
              key={alarm.id}
              alarm={alarm}
              cameraName={cameraNameMap?.[alarm.cameraId]}
              isSelected={selectedAlarmIds.has(alarm.id)}
              onToggleSelect={onToggleSelectAlarm}
              onSelect={onSelect}
              onSelectCrop={onSelectCrop}
              onToggleStatus={onToggleStatus}
              t={t}
            />
          ))}
        </tbody>
      </table>
    </div>
  )
}
