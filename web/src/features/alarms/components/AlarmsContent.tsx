import React from 'react'
import { AlertCircle } from 'lucide-react'
import type { AlarmRecord } from '../../../types'
import { AlarmCardItem } from './AlarmCardItem'
import { AlarmTableRow } from './AlarmTableRow'

export type ViewMode = 'cards' | 'table'

export interface AlarmsContentProps {
  alarms: AlarmRecord[]
  viewMode: ViewMode
  cameraNameMap?: Record<string, string>
  selectedAlarmIds: Set<number>
  onToggleSelectAlarm: (id: number, selected: boolean) => void
  onToggleSelectAll: (selected: boolean) => void
  onSelect: (alarm: AlarmRecord) => void
  onSelectCrop: (alarm: AlarmRecord) => void
  onToggleStatus: (alarm: AlarmRecord) => void
  t: (key: string) => string
}

export function AlarmsContent({
  alarms,
  viewMode,
  cameraNameMap,
  selectedAlarmIds,
  onToggleSelectAlarm,
  onToggleSelectAll,
  onSelect,
  onSelectCrop,
  onToggleStatus,
  t,
}: AlarmsContentProps): React.ReactElement {
  if (alarms.length === 0) {
    return (
      <div className="py-24 text-center text-[var(--text-muted)]">
        <AlertCircle className="mx-auto mb-2 h-8 w-8 opacity-40" />
        <p className="font-medium text-[var(--text-secondary)]">{t('empty.alarms')}</p>
        <p className="text-xs opacity-75">{t('empty.alarmsDesc')}</p>
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
            {alarms.length} {t('columns.evidence')}
          </span>
        </div>

        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
          {alarms.map((alarm) => (
            <AlarmCardItem
              key={alarm.id}
              alarm={alarm}
              cameraName={cameraNameMap?.[alarm.cameraId]}
              isSelected={selectedAlarmIds.has(alarm.id)}
              onToggleSelect={(selected) => onToggleSelectAlarm(alarm.id, selected)}
              onSelect={() => onSelect(alarm)}
              onSelectCrop={() => onSelectCrop(alarm)}
              onToggleStatus={() => onToggleStatus(alarm)}
              t={t}
            />
          ))}
        </div>
      </div>
    )
  }

  return (
    <div className="overflow-x-auto rounded-xl border border-[var(--border)]">
      <table className="w-full text-left text-xs text-[var(--text-secondary)]">
        <thead className="border-b border-[var(--border)] bg-[var(--bg-secondary)] text-[11px] font-semibold text-[var(--text-muted)] uppercase">
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
              onToggleSelect={(selected) => onToggleSelectAlarm(alarm.id, selected)}
              onSelect={() => onSelect(alarm)}
              onSelectCrop={() => onSelectCrop(alarm)}
              onToggleStatus={() => onToggleStatus(alarm)}
              t={t}
            />
          ))}
        </tbody>
      </table>
    </div>
  )
}
