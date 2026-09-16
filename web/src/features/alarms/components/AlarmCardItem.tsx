import React from 'react'
import { Clock, ExternalLink } from 'lucide-react'
import { evidenceApi } from '../../../lib/api'
import type { AlarmRecord } from '../../../types'
import { formatTimestamp, getRuleTypeLabel, preloadImage } from '../utils'
import { AlarmStatusButton } from './AlarmStatusButton'

export interface AlarmCardItemProps {
  alarm: AlarmRecord
  cameraName?: string
  isSelected?: boolean
  onToggleSelect?: (selected: boolean) => void
  onSelect: () => void
  onSelectCrop: () => void
  onToggleStatus: () => void
  t: (key: string) => string
}

export function AlarmCardItem({
  alarm,
  cameraName,
  isSelected = false,
  onToggleSelect,
  onSelect,
  onSelectCrop,
  onToggleStatus,
  t,
}: AlarmCardItemProps): React.ReactElement {
  const isProcessed = alarm.status === 'processed'

  return (
    <div
      onClick={onSelect}
      onPointerEnter={() => preloadImage(evidenceApi.getImageUrl(alarm.imageRelPath))}
      onTouchStart={() => preloadImage(evidenceApi.getImageUrl(alarm.imageRelPath))}
      className={`group relative flex cursor-pointer flex-col overflow-hidden rounded-2xl border bg-[var(--bg-surface)] shadow-xs transition-all duration-200 hover:shadow-md ${
        isSelected
          ? 'border-[var(--accent)] ring-1 ring-[var(--accent)]/50'
          : isProcessed
            ? 'border-[var(--border)] opacity-75'
            : 'border-rose-500/30 hover:border-rose-500/70'
      }`}
    >
      <div className="relative aspect-video w-full overflow-hidden bg-black/90">
        {/* 多选勾选复选框 */}
        {onToggleSelect && (
          <div className="absolute top-2 right-2 z-20" onClick={(e) => e.stopPropagation()}>
            <input
              type="checkbox"
              checked={isSelected}
              onChange={(e) => onToggleSelect(e.target.checked)}
              className="h-4 w-4 cursor-pointer rounded border-[var(--border)] bg-black/60 text-[var(--accent)] transition-transform hover:scale-110 focus:ring-1 focus:ring-[var(--accent)]"
              aria-label={`Select alarm ${alarm.eventId}`}
            />
          </div>
        )}

        {alarm.cropImageRelPath || alarm.imageRelPath ? (
          <img
            src={evidenceApi.getImageUrl(alarm.cropImageRelPath || alarm.imageRelPath)}
            alt={alarm.eventId}
            loading="lazy"
            decoding="async"
            className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center font-mono text-xs text-slate-500">
            {t('card.noImage')}
          </div>
        )}

        {alarm.cropImageRelPath && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onSelectCrop()
            }}
            className="absolute right-2 bottom-2 z-20 flex items-center gap-1 rounded-lg border border-white/40 bg-black/80 px-2 py-1 text-[10px] font-medium text-white shadow-md backdrop-blur-xs transition-all duration-200 hover:border-white/70 hover:shadow-lg hover:shadow-black/40"
            title={t('modal.cropImage')}
          >
            <ExternalLink className="h-3 w-3" />
            <span>{t('card.siteCrop')}</span>
          </button>
        )}

        <div className="absolute top-2 left-2 flex items-center gap-1.5">
          <span
            className={`rounded-md px-2 py-0.5 text-[10px] font-bold tracking-wider uppercase shadow-xs backdrop-blur-md ${
              alarm.severity === 'critical'
                ? 'bg-rose-500/80 text-white'
                : 'bg-amber-500/80 text-white'
            }`}
          >
            {alarm.severity || 'WARNING'}
          </span>
          <span className="rounded-md bg-black/60 px-1.5 py-0.5 font-mono text-[10px] text-white backdrop-blur-xs">
            {getRuleTypeLabel(alarm.ruleType, t)}
          </span>
        </div>
      </div>

      <div className="space-y-2 p-3.5 text-xs">
        <div className="flex items-center justify-between">
          <span className="font-semibold text-[var(--text-primary)]">
            {t('card.target')}: {alarm.targetLabel}
          </span>
          <span className="font-mono text-[11px] font-semibold text-[var(--accent)]">
            {((alarm.confidence ?? 0) * 100).toFixed(0)}% {t('card.confidence')}
          </span>
        </div>

        <div className="flex items-center justify-between font-mono text-[11px] text-[var(--text-muted)]">
          <span
            className="max-w-[140px] truncate font-sans font-medium text-[var(--text-secondary)]"
            title={cameraName || alarm.cameraId}
          >
            {cameraName || alarm.cameraId}
          </span>
          <span className="flex items-center gap-1">
            <Clock className="h-3 w-3 shrink-0" />
            {formatTimestamp(alarm.occurredAt)}
          </span>
        </div>

        <div className="flex items-center justify-between border-t border-[var(--border)] pt-2">
          <AlarmStatusButton
            isProcessed={isProcessed}
            onClick={(e) => {
              e.stopPropagation()
              onToggleStatus()
            }}
            t={t}
          />

          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onSelect()
            }}
            className="flex items-center gap-1 text-[11px] text-[var(--text-muted)] transition-all hover:text-[var(--text-primary)]"
          >
            <span>{t('card.viewHd')}</span>
            <ExternalLink className="h-3 w-3" />
          </button>
        </div>
      </div>
    </div>
  )
}
