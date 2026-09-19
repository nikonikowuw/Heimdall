import React from 'react'
import { Camera as CameraIcon, Clock, RotateCcw, Search } from 'lucide-react'
import { evidenceApi } from '@/lib/api'
import type { CaptureRecord } from '@/types'
import type { ViewMode } from './AlarmsContent'
import { formatTimestamp, preloadImage } from '../utils'

export interface CaptureCardItemProps {
  capture: CaptureRecord
  cameraName?: string
  onSelect: () => void
  t: (key: string) => string
}

export function CaptureCardItem({
  capture,
  cameraName,
  onSelect,
  t,
}: CaptureCardItemProps): React.ReactElement {
  return (
    <div
      onClick={onSelect}
      onPointerEnter={() => preloadImage(evidenceApi.getImageUrl(capture.imageRelPath))}
      onTouchStart={() => preloadImage(evidenceApi.getImageUrl(capture.imageRelPath))}
      className="group relative flex cursor-pointer flex-col overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-surface)] transition-all duration-200 hover:border-cyan-500/50 hover:shadow-md"
    >
      <div className="relative aspect-square w-full overflow-hidden bg-black/90">
        {capture.cropImageRelPath || capture.imageRelPath ? (
          <img
            src={evidenceApi.getImageUrl(capture.cropImageRelPath || capture.imageRelPath)}
            alt={capture.captureId}
            loading="lazy"
            decoding="async"
            className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center font-mono text-xs text-slate-500">
            {t('card.noImage')}
          </div>
        )}

        <div className="absolute top-2 left-2 flex items-center gap-1.5">
          <span className="rounded-md bg-cyan-500/80 px-2 py-0.5 font-mono text-[10px] font-bold text-white shadow-xs backdrop-blur-md">
            #{capture.trackId}
          </span>
          <span className="rounded-md bg-black/60 px-1.5 py-0.5 font-mono text-[10px] text-white backdrop-blur-xs">
            {capture.targetLabel}
          </span>
        </div>
      </div>

      <div className="space-y-1.5 p-3 text-xs">
        <div className="flex items-center justify-between font-medium">
          <span
            className="max-w-[120px] truncate text-[var(--text-primary)]"
            title={cameraName || capture.cameraId}
          >
            {cameraName || capture.cameraId}
          </span>
          <span className="font-mono text-[11px] text-cyan-400">
            {(capture.confidence * 100).toFixed(0)}%
          </span>
        </div>
        <div className="flex items-center gap-1 font-mono text-[10px] text-[var(--text-muted)]">
          <Clock className="h-3 w-3" />
          <span>{formatTimestamp(capture.capturedAt)}</span>
        </div>
      </div>
    </div>
  )
}

export interface CapturesContentProps {
  captures: CaptureRecord[]
  viewMode?: ViewMode
  cameraNameMap?: Record<string, string>
  hasActiveFilters?: boolean
  searchQuery?: string
  onResetFilters?: () => void
  onClearSearch?: () => void
  onSelect: (capture: CaptureRecord) => void
  t: (key: string, options?: Record<string, unknown>) => string
}

export function CapturesContent({
  captures,
  viewMode = 'cards',
  cameraNameMap,
  hasActiveFilters = false,
  searchQuery,
  onResetFilters,
  onClearSearch,
  onSelect,
  t,
}: CapturesContentProps): React.ReactElement {
  if (captures.length === 0) {
    if (searchQuery && searchQuery.trim()) {
      return (
        <div className="flex flex-col items-center justify-center py-24 text-center text-[var(--text-muted)]">
          <Search className="mx-auto mb-2 h-8 w-8 opacity-40" />
          <p className="font-medium text-[var(--text-secondary)]">
            {t('search.noMatch', { query: searchQuery.trim() })}
          </p>
          {onClearSearch && (
            <button
              type="button"
              onClick={onClearSearch}
              className="mt-4 flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-1.5 text-xs font-medium text-[var(--accent)] shadow-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)]/20 active:scale-95"
            >
              <RotateCcw className="h-3.5 w-3.5" />
              <span>{t('search.clearQuery')}</span>
            </button>
          )}
        </div>
      )
    }

    return (
      <div className="flex flex-col items-center justify-center py-24 text-center text-[var(--text-muted)]">
        <CameraIcon className="mx-auto mb-2 h-8 w-8 opacity-40" />
        <p className="font-medium text-[var(--text-secondary)]">{t('empty.captures')}</p>
        {hasActiveFilters && onResetFilters && (
          <button
            type="button"
            onClick={onResetFilters}
            className="mt-4 flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-1.5 text-xs font-medium text-[var(--accent)] shadow-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)]/20 active:scale-95"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            <span>{t('empty.resetFilter')}</span>
          </button>
        )}
      </div>
    )
  }

  if (viewMode === 'table') {
    return (
      <div className="w-full overflow-x-auto">
        <table className="w-full text-left text-xs text-[var(--text-secondary)]">
          <thead className="sticky top-0 z-10 border-b border-[var(--border)]/70 bg-[var(--bg-secondary)]/80 text-[11px] font-semibold tracking-wider text-[var(--text-muted)] uppercase backdrop-blur-md">
            <tr>
              <th className="px-3.5 py-3">{t('columns.thumbnail')}</th>
              <th className="px-3.5 py-3">Track ID</th>
              <th className="px-3.5 py-3">{t('columns.targetLabel')}</th>
              <th className="px-3.5 py-3">{t('columns.confidence')}</th>
              <th className="px-3.5 py-3">{t('columns.camera')}</th>
              <th className="px-3.5 py-3">{t('columns.occurredAt')}</th>
              <th className="px-3.5 py-3 text-right">{t('columns.actions')}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-[var(--border)]/40">
            {captures.map((capture) => (
              <tr
                key={capture.id}
                onClick={() => onSelect(capture)}
                className="group cursor-pointer transition-colors hover:bg-[var(--accent-soft)]/20"
              >
                <td className="px-3.5 py-2.5">
                  <div className="h-10 w-10 overflow-hidden rounded-xl border border-[var(--border)] bg-black shadow-xs">
                    <img
                      src={evidenceApi.getImageUrl(
                        capture.cropImageRelPath || capture.imageRelPath,
                      )}
                      alt={capture.captureId}
                      loading="lazy"
                      decoding="async"
                      className="h-full w-full object-cover transition-transform duration-200 group-hover:scale-105"
                    />
                  </div>
                </td>
                <td className="px-3.5 py-2.5 font-mono text-xs font-semibold text-cyan-500">
                  #{capture.trackId}
                </td>
                <td className="px-3.5 py-2.5">
                  <span className="rounded-md border border-[var(--border)] bg-[var(--bg-secondary)]/60 px-2 py-0.5 font-mono text-[11px] text-[var(--text-primary)]">
                    {capture.targetLabel}
                  </span>
                </td>
                <td className="px-3.5 py-2.5 font-mono text-xs font-semibold text-cyan-400">
                  {(capture.confidence * 100).toFixed(0)}%
                </td>
                <td className="px-3.5 py-2.5 text-xs text-[var(--text-primary)]">
                  {cameraNameMap?.[capture.cameraId] || capture.cameraId}
                </td>
                <td className="px-3.5 py-2.5 font-mono text-[11px] text-[var(--text-muted)]">
                  {formatTimestamp(capture.capturedAt)}
                </td>
                <td className="px-3.5 py-2.5 text-right">
                  <span className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1 text-[11px] font-medium text-[var(--text-secondary)] shadow-2xs transition-all group-hover:border-cyan-500 group-hover:text-cyan-400">
                    {t('viewImage')}
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    )
  }

  return (
    <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6">
      {captures.map((capture) => (
        <CaptureCardItem
          key={capture.id}
          capture={capture}
          cameraName={cameraNameMap?.[capture.cameraId]}
          onSelect={() => onSelect(capture)}
          t={t}
        />
      ))}
    </div>
  )
}
