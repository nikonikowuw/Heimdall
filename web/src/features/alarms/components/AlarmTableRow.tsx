import React from 'react'
import { evidenceApi } from '../../../lib/api'
import type { AlarmRecord } from '../../../types'
import { formatTimestamp, getRuleTypeLabel } from '../utils'

export interface AlarmTableRowProps {
  alarm: AlarmRecord
  cameraName?: string
  isSelected?: boolean
  onToggleSelect?: (selected: boolean) => void
  onSelect: () => void
  onSelectCrop: () => void
  onToggleStatus: () => void
  t: (key: string) => string
}

export function AlarmTableRow({
  alarm,
  cameraName,
  isSelected = false,
  onToggleSelect,
  onSelect,
  onSelectCrop,
  onToggleStatus,
  t,
}: AlarmTableRowProps): React.ReactElement {
  const isProcessed = alarm.status === 'processed'

  return (
    <tr
      onClick={onSelect}
      className={`cursor-pointer transition-colors hover:bg-[var(--accent-soft)]/20 ${
        isSelected ? 'bg-[var(--accent-soft)]/30' : ''
      }`}
    >
      {onToggleSelect && (
        <td className="px-3 py-2" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={isSelected}
            onChange={(e) => onToggleSelect(e.target.checked)}
            className="h-4 w-4 cursor-pointer rounded border-[var(--border)] bg-[var(--bg-surface)] text-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]"
            aria-label={`Select alarm ${alarm.eventId}`}
          />
        </td>
      )}

      <td className="px-3 py-2">
        {alarm.cropImageRelPath || alarm.imageRelPath ? (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              if (alarm.cropImageRelPath) {
                onSelectCrop()
              } else {
                onSelect()
              }
            }}
            className="h-10 w-16 shrink-0 overflow-hidden rounded border border-[var(--border)] bg-black/80 transition-all duration-200 hover:border-[var(--accent)]/50 hover:shadow-md"
            title={alarm.cropImageRelPath ? t('modal.cropImage') : t('modal.fullImage')}
          >
            <img
              src={evidenceApi.getImageUrl(alarm.cropImageRelPath || alarm.imageRelPath)}
              alt="Thumb"
              className="h-full w-full object-cover"
            />
          </button>
        ) : (
          <div className="flex h-10 w-16 items-center justify-center rounded border border-[var(--border)] bg-black/80 font-mono text-[9px] text-slate-500">
            N/A
          </div>
        )}
      </td>
      <td className="px-3 py-2 font-mono text-[11px] text-[var(--text-primary)]">
        {alarm.eventId.slice(0, 12)}...
      </td>
      <td className="px-3 py-2 font-medium text-[var(--text-primary)]">
        {cameraName || alarm.cameraId}
      </td>
      <td className="px-3 py-2 font-semibold text-[var(--text-primary)]">{alarm.targetLabel}</td>
      <td className="px-3 py-2 font-mono text-[11px]">{getRuleTypeLabel(alarm.ruleType, t)}</td>
      <td className="px-3 py-2">
        <span
          className={`rounded px-1.5 py-0.5 text-[10px] font-bold uppercase ${
            alarm.severity === 'critical'
              ? 'bg-rose-500/20 text-rose-500'
              : 'bg-amber-500/20 text-amber-500'
          }`}
        >
          {alarm.severity || 'WARNING'}
        </span>
      </td>
      <td className="px-3 py-2 font-mono text-[11px]">
        {((alarm.confidence ?? 0) * 100).toFixed(0)}%
      </td>
      <td className="px-3 py-2">
        <span
          className={`rounded px-2 py-0.5 text-[10px] font-semibold ${
            isProcessed
              ? 'border border-emerald-500/30 bg-emerald-500/15 text-emerald-500'
              : 'border border-rose-500/30 bg-rose-500/15 text-rose-500'
          }`}
        >
          {isProcessed ? t('card.processed') : t('statusFilter.unprocessed')}
        </span>
      </td>
      <td className="px-3 py-2 font-mono text-[11px] text-[var(--text-muted)]">
        {formatTimestamp(alarm.occurredAt)}
      </td>
      <td className="px-3 py-2 text-right">
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation()
            onToggleStatus()
          }}
          className={`rounded px-2.5 py-1 text-[11px] font-semibold transition-all ${
            isProcessed
              ? 'bg-emerald-500/15 text-emerald-500 hover:bg-emerald-500/25'
              : 'bg-rose-500 text-white hover:opacity-90'
          }`}
        >
          {isProcessed ? t('card.processed') : t('card.markProcessed')}
        </button>
      </td>
    </tr>
  )
}
