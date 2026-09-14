import React, { useEffect, useRef, useState } from 'react'
import { Camera as CameraIcon, Download, Loader2, RefreshCw, X } from 'lucide-react'
import { useDismissStack } from '../../../hooks/use-dismiss-stack'
import { evidenceApi } from '../../../lib/api'
import type { CaptureRecord } from '../../../types'
import {
  calculateFittedImageRect,
  type FittedImageRect,
  formatTimestamp,
  parseBBoxCoords,
} from '../utils'

export interface CaptureLightboxModalProps {
  capture: CaptureRecord
  cameraName?: string
  onClose: () => void
  t: (key: string) => string
}

type FullImageStatus = 'unavailable' | 'loading' | 'loaded' | 'error'

export function CaptureLightboxModal({
  capture,
  cameraName,
  onClose,
  t,
}: CaptureLightboxModalProps): React.ReactElement {
  const containerRef = useRef<HTMLDivElement>(null)
  const fullImageUrl = capture.imageRelPath ? evidenceApi.getImageUrl(capture.imageRelPath) : ''
  const previousFullImageUrl = useRef(fullImageUrl)
  const [fullImageStatus, setFullImageStatus] = useState<FullImageStatus>(
    fullImageUrl ? 'loading' : 'unavailable',
  )
  const [imageAttempt, setImageAttempt] = useState<number>(0)
  const [imgRect, setImgRect] = useState<FittedImageRect | null>(null)
  const isFullLoaded = fullImageStatus === 'loaded'
  const isFullLoading = fullImageStatus === 'loading'
  const isFullError = fullImageStatus === 'error'
  const fullImageSrc =
    imageAttempt > 0 && fullImageUrl
      ? `${fullImageUrl}${fullImageUrl.includes('?') ? '&' : '?'}retry=${imageAttempt}`
      : fullImageUrl
  const bbox = parseBBoxCoords(capture.bboxJson)

  useEffect(() => {
    if (previousFullImageUrl.current === fullImageUrl) return
    previousFullImageUrl.current = fullImageUrl
    setFullImageStatus(fullImageUrl ? 'loading' : 'unavailable')
    setImageAttempt(0)
    setImgRect(null)
  }, [fullImageUrl])

  // 支持 ESC 浮层栈快速退出
  useDismissStack(true, onClose)

  const handleImageLoad = (e: React.SyntheticEvent<HTMLImageElement>) => {
    setFullImageStatus('loaded')
    const container = containerRef.current
    if (!container) return
    const img = e.currentTarget
    setImgRect(
      calculateFittedImageRect(
        container.clientWidth,
        container.clientHeight,
        img.naturalWidth,
        img.naturalHeight,
      ),
    )
  }

  const handleImageError = () => {
    setFullImageStatus('error')
    setImgRect(null)
  }

  const handleRetry = () => {
    setImageAttempt((attempt) => attempt + 1)
    setFullImageStatus('loading')
  }

  const retryButton = (
    <button
      type="button"
      onClick={handleRetry}
      className="flex items-center gap-1.5 rounded-lg border border-white/20 px-2 py-1 text-[11px] text-white transition-colors hover:bg-white/10"
    >
      <RefreshCw className="h-3 w-3" />
      <span>{t('modal.retryImage')}</span>
    </button>
  )

  return (
    <div
      className="animate-in fade-in fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4 backdrop-blur-sm duration-150"
      onClick={onClose}
    >
      <div
        className="relative flex max-h-[94vh] w-full max-w-5xl flex-col overflow-hidden rounded-3xl border border-[var(--border)] bg-[var(--bg-surface)] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-[var(--border)] bg-[var(--bg-secondary)]/50 px-6 py-4">
          <div className="flex items-center gap-2">
            <CameraIcon className="h-5 w-5 text-cyan-400" />
            <h3 className="text-sm font-semibold text-[var(--text-primary)]">
              {capture.targetLabel} (Track #{capture.trackId})
            </h3>
          </div>

          <div className="flex items-center gap-2">
            {capture.imageRelPath && (
              <a
                href={evidenceApi.getImageUrl(capture.imageRelPath)}
                download={`capture_${capture.captureId}.jpg`}
                target="_blank"
                rel="noreferrer"
                className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 text-xs text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
              >
                <Download className="h-3.5 w-3.5" />
                <span className="hidden sm:inline">{t('modal.fullImage')}</span>
              </a>
            )}
            <button
              type="button"
              onClick={onClose}
              className="rounded-xl p-1.5 text-[var(--text-muted)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
            >
              <X className="h-5 w-5" />
            </button>
          </div>
        </div>

        <div className="flex-1 space-y-4 overflow-auto p-6">
          <div
            ref={containerRef}
            className="relative flex aspect-video w-full items-center justify-center overflow-hidden rounded-2xl border border-[var(--border)] bg-black shadow-md select-none"
          >
            {fullImageStatus !== 'loaded' && capture.cropImageRelPath && (
              <div className="absolute inset-0 flex flex-col items-center justify-center overflow-hidden bg-black/60 backdrop-blur-md">
                <img
                  src={evidenceApi.getImageUrl(capture.cropImageRelPath)}
                  alt="Preview Placeholder"
                  className="max-h-[70%] max-w-[70%] rounded-xl object-contain opacity-75 shadow-2xl transition-opacity duration-300"
                />
                {isFullLoading && (
                  <div className="absolute bottom-4 flex items-center gap-2 rounded-full border border-white/20 bg-black/70 px-3 py-1 text-[11px] text-slate-200 shadow-lg backdrop-blur-md">
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-cyan-400" />
                    <span>{t('modal.loadingFullHd')}</span>
                  </div>
                )}
                {isFullError && (
                  <div className="absolute inset-x-4 bottom-4 flex flex-wrap items-center justify-center gap-2 rounded-lg border border-cyan-400/30 bg-black/75 px-3 py-2 text-[11px] text-slate-200 shadow-lg">
                    <span>{t('modal.fullImageLoadFailed')}</span>
                    {retryButton}
                  </div>
                )}
              </div>
            )}

            {isFullLoading && !capture.cropImageRelPath && (
              <div className="absolute inset-0 flex items-center justify-center">
                <Loader2 className="h-8 w-8 animate-spin text-cyan-400 opacity-60" />
              </div>
            )}

            {isFullError && !capture.cropImageRelPath && (
              <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-black/75 p-4 text-center text-xs text-slate-200">
                <span>{t('modal.fullImageLoadFailed')}</span>
                {retryButton}
              </div>
            )}

            {capture.imageRelPath ? (
              <>
                <img
                  src={fullImageSrc}
                  alt="Capture"
                  className={`h-full w-full object-contain transition-opacity duration-300 ${
                    isFullLoaded ? 'opacity-100' : 'opacity-0'
                  }`}
                  onLoad={handleImageLoad}
                  onError={handleImageError}
                />
                {bbox && imgRect && isFullLoaded && (
                  <div
                    className="pointer-events-none absolute border-2 border-cyan-400 bg-cyan-400/15 shadow-[0_0_12px_rgba(6,182,212,0.5)] transition-all"
                    style={{
                      left: `${imgRect.x + bbox.x1 * imgRect.width}px`,
                      top: `${imgRect.y + bbox.y1 * imgRect.height}px`,
                      width: `${Math.max(6, (bbox.x2 - bbox.x1) * imgRect.width)}px`,
                      height: `${Math.max(6, (bbox.y2 - bbox.y1) * imgRect.height)}px`,
                    }}
                  >
                    <span className="absolute -top-5 left-0 rounded bg-cyan-500 px-1.5 py-0.5 font-mono text-[9px] font-bold whitespace-nowrap text-black shadow-xs">
                      #{capture.trackId} {capture.targetLabel} (
                      {(capture.confidence * 100).toFixed(0)}%)
                    </span>
                  </div>
                )}
              </>
            ) : !capture.cropImageRelPath ? (
              <div className="flex h-full w-full items-center justify-center font-mono text-sm text-slate-500">
                {t('modal.noImage')}
              </div>
            ) : null}
          </div>

          <div className="flex flex-wrap items-center justify-between gap-4 rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4">
            <div className="flex items-center gap-4">
              {capture.cropImageRelPath && (
                <div className="h-16 w-16 overflow-hidden rounded-xl border border-white/30 bg-black/80 shadow-xs">
                  <img
                    src={evidenceApi.getImageUrl(capture.cropImageRelPath)}
                    alt="Crop"
                    className="h-full w-full object-cover"
                  />
                </div>
              )}
              <div className="space-y-1 text-xs">
                <div className="font-semibold text-[var(--text-primary)]">
                  {t('modal.cropImage')}
                </div>
                <div className="font-mono text-[11px] text-[var(--text-muted)]">
                  {t('modal.channel')}: {cameraName || capture.cameraId} · {t('modal.trackId')}: #
                  {capture.trackId}
                </div>
                <div className="font-mono text-[11px] text-[var(--text-muted)]">
                  {t('modal.time')}: {formatTimestamp(capture.capturedAt)}
                </div>
              </div>
            </div>

            <div className="flex items-center gap-3">
              <span className="rounded-xl border border-cyan-500/30 bg-cyan-500/10 px-3 py-1.5 font-mono text-xs font-semibold text-cyan-400">
                {t('modal.qualityScore')}: {capture.qualityScore.toFixed(2)}
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
