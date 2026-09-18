import React from 'react'
import { Minus, Plus, RotateCcw } from 'lucide-react'

export interface ZoomControlsProps {
  zoom: number
  onZoomIn: () => void
  onZoomOut: () => void
  onResetZoom: () => void
  minZoom?: number
  maxZoom?: number
  t: (key: string, options?: Record<string, unknown>) => string
  className?: string
  variant?: 'glass' | 'dark'
}

export function ZoomControls({
  zoom,
  onZoomIn,
  onZoomOut,
  onResetZoom,
  minZoom = 1,
  maxZoom = 5,
  t,
  className = '',
  variant = 'glass',
}: ZoomControlsProps): React.ReactElement {
  const isDark = variant === 'dark'

  return (
    <div
      className={`flex items-center gap-1 rounded-xl border px-2 py-1 shadow-sm select-none ${
        isDark
          ? 'border-white/20 bg-black/75 backdrop-blur-md'
          : 'border-white/15 bg-white/5 backdrop-blur-xs'
      } ${className}`}
      onClick={(e) => e.stopPropagation()}
    >
      <button
        type="button"
        onClick={onZoomOut}
        disabled={zoom <= minZoom}
        className="rounded p-1 text-zinc-300 transition-colors hover:bg-white/15 hover:text-white disabled:opacity-30"
        title={t('modal.zoomOut')}
        aria-label={t('modal.zoomOut')}
      >
        <Minus className="h-3.5 w-3.5" />
      </button>
      <span className="min-w-[42px] text-center font-mono text-[11px] font-semibold text-white tabular-nums">
        {Math.round(zoom * 100)}%
      </span>
      <button
        type="button"
        onClick={onZoomIn}
        disabled={zoom >= maxZoom}
        className="rounded p-1 text-zinc-300 transition-colors hover:bg-white/15 hover:text-white disabled:opacity-30"
        title={t('modal.zoomIn')}
        aria-label={t('modal.zoomIn')}
      >
        <Plus className="h-3.5 w-3.5" />
      </button>
      {zoom > minZoom && (
        <button
          type="button"
          onClick={onResetZoom}
          className="ml-1 border-l border-white/20 pl-1 text-zinc-300 transition-colors hover:text-white"
          title={t('modal.zoomReset')}
          aria-label={t('modal.zoomReset')}
        >
          <RotateCcw className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  )
}
