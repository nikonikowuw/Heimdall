import React from 'react'
import { Camera as CameraIcon, Clock } from 'lucide-react'
import { evidenceApi } from '../../../lib/api'
import type { CaptureRecord } from '../../../types'
import { formatTimestamp } from '../utils'

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
      className="group relative flex cursor-pointer flex-col overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-surface)] transition-all duration-200 hover:border-cyan-500/50 hover:shadow-md"
    >
      <div className="relative aspect-square w-full overflow-hidden bg-black/90">
        {capture.cropImageRelPath || capture.imageRelPath ? (
          <img
            src={evidenceApi.getImageUrl(capture.cropImageRelPath || capture.imageRelPath)}
            alt={capture.captureId}
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
  cameraNameMap?: Record<string, string>
  onSelect: (capture: CaptureRecord) => void
  t: (key: string) => string
}

export function CapturesContent({
  captures,
  cameraNameMap,
  onSelect,
  t,
}: CapturesContentProps): React.ReactElement {
  if (captures.length === 0) {
    return (
      <div className="py-24 text-center text-[var(--text-muted)]">
        <CameraIcon className="mx-auto mb-2 h-8 w-8 opacity-40" />
        <p className="font-medium text-[var(--text-secondary)]">{t('empty.captures')}</p>
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
