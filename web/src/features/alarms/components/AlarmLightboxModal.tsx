import React, { useEffect, useRef, useState } from 'react'
import { Check, CheckCircle2, Download, Loader2, RefreshCw, ShieldAlert, X } from 'lucide-react'
import { useDismissStack } from '../../../hooks/use-dismiss-stack'
import { evidenceApi } from '../../../lib/api'
import type { AlarmRecord } from '../../../types'
import {
  calculateFittedImageRect,
  formatFaceBBoxLabel,
  getBBoxStyle,
  type FittedImageRect,
  formatTimestamp,
  getRuleTypeLabel,
  parseTargetBBoxes,
} from '../utils'

export interface AlarmLightboxModalProps {
  alarm: AlarmRecord
  cameraName?: string
  onClose: () => void
  onToggleStatus: () => void
  onSelectCrop?: () => void
  t: (key: string) => string
}

type FullImageStatus = 'unavailable' | 'loading' | 'loaded' | 'error'

export function AlarmLightboxModal({
  alarm,
  cameraName,
  onClose,
  onToggleStatus,
  onSelectCrop,
  t,
}: AlarmLightboxModalProps): React.ReactElement {
  const isProcessed = alarm.status === 'processed'
  const containerRef = useRef<HTMLDivElement>(null)
  const fullImageUrl = alarm.imageRelPath ? evidenceApi.getImageUrl(alarm.imageRelPath) : ''
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

  const targetBBoxes = parseTargetBBoxes(alarm.bboxJson)
  const bodyBBox = targetBBoxes?.body ?? null
  const faceBBox = targetBBoxes?.face?.bbox ?? null
  const faceConfidence = targetBBoxes?.face?.confidence

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
        {/* 顶部标题栏 */}
        <div className="flex items-center justify-between border-b border-[var(--border)] bg-[var(--bg-secondary)]/50 px-6 py-4">
          <div className="flex items-center gap-2.5">
            <ShieldAlert
              className={`h-5 w-5 ${
                alarm.severity === 'critical' ? 'text-rose-500' : 'text-amber-500'
              }`}
            />
            <h3 className="text-sm font-semibold text-[var(--text-primary)]">
              {alarm.targetLabel} · {getRuleTypeLabel(alarm.ruleType, t)}
            </h3>
            <span className="rounded-md border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-0.5 font-mono text-xs text-[var(--text-muted)]">
              {alarm.eventId}
            </span>
          </div>

          <div className="flex items-center gap-2">
            {alarm.imageRelPath && (
              <a
                href={evidenceApi.getImageUrl(alarm.imageRelPath)}
                download={`alarm_${alarm.eventId}.jpg`}
                target="_blank"
                rel="noreferrer"
                className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 text-xs text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                title={t('viewImage')}
              >
                <Download className="h-3.5 w-3.5" />
                <span className="hidden sm:inline">{t('modal.fullImage')}</span>
              </a>
            )}
            <button
              type="button"
              onClick={onClose}
              className="rounded-xl p-1.5 text-[var(--text-muted)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
              title="Close (Esc)"
            >
              <X className="h-5 w-5" />
            </button>
          </div>
        </div>

        {/* 证据大图与高精度 BBox 检测框 */}
        <div className="flex-1 space-y-4 overflow-auto p-6">
          <div
            ref={containerRef}
            className="relative flex aspect-video w-full items-center justify-center overflow-hidden rounded-2xl border border-[var(--border)] bg-black shadow-md select-none"
          >
            {fullImageStatus !== 'loaded' && alarm.cropImageRelPath && (
              <div className="absolute inset-0 flex flex-col items-center justify-center overflow-hidden bg-black/60 backdrop-blur-md">
                <img
                  src={evidenceApi.getImageUrl(alarm.cropImageRelPath)}
                  alt="Preview Placeholder"
                  className="max-h-[70%] max-w-[70%] rounded-xl object-contain opacity-75 shadow-2xl transition-opacity duration-150"
                />
                {isFullLoading && (
                  <div className="absolute bottom-4 flex items-center gap-2 rounded-full border border-white/20 bg-black/70 px-3 py-1 text-[11px] text-slate-200 shadow-lg backdrop-blur-md">
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-rose-500" />
                    <span>{t('modal.loadingFullHd')}</span>
                  </div>
                )}
                {isFullError && (
                  <div className="absolute inset-x-4 bottom-4 flex flex-wrap items-center justify-center gap-2 rounded-lg border border-rose-400/30 bg-black/75 px-3 py-2 text-[11px] text-slate-200 shadow-lg">
                    <span>{t('modal.fullImageLoadFailed')}</span>
                    {retryButton}
                  </div>
                )}
              </div>
            )}

            {isFullLoading && !alarm.cropImageRelPath && (
              <div className="absolute inset-0 flex items-center justify-center">
                <Loader2 className="h-8 w-8 animate-spin text-rose-500 opacity-60" />
              </div>
            )}

            {isFullError && !alarm.cropImageRelPath && (
              <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-black/75 p-4 text-center text-xs text-slate-200">
                <span>{t('modal.fullImageLoadFailed')}</span>
                {retryButton}
              </div>
            )}

            {alarm.imageRelPath ? (
              <>
                <img
                  src={fullImageSrc}
                  alt="Full Frame"
                  fetchPriority="high"
                  decoding="async"
                  className={`h-full w-full object-contain transition-opacity duration-150 ${
                    isFullLoaded ? 'opacity-100' : 'opacity-0'
                  }`}
                  onLoad={handleImageLoad}
                  onError={handleImageError}
                />
                {bodyBBox && imgRect && isFullLoaded && (
                  <div
                    className="pointer-events-none absolute border-2 border-rose-500 bg-rose-500/15 shadow-[0_0_15px_rgba(244,63,94,0.6)] transition-all"
                    style={getBBoxStyle(bodyBBox, imgRect)}
                  >
                    <span className="absolute -top-6 left-0 rounded bg-rose-600 px-2 py-0.5 font-mono text-[10px] font-bold whitespace-nowrap text-white shadow-xs">
                      #{alarm.trackId} {alarm.targetLabel} (
                      {((alarm.confidence ?? 0) * 100).toFixed(0)}%)
                    </span>
                  </div>
                )}
                {faceBBox && imgRect && isFullLoaded && (
                  <div
                    className="pointer-events-none absolute border-2 border-dashed border-purple-400 bg-purple-500/15 shadow-[0_0_12px_rgba(168,85,247,0.5)] transition-all"
                    style={getBBoxStyle(faceBBox, imgRect)}
                  >
                    <span className="absolute -top-4.5 left-0 rounded bg-purple-600 px-1.5 py-0.5 font-mono text-[8px] font-bold whitespace-nowrap text-white shadow-xs">
                      {formatFaceBBoxLabel(targetBBoxes?.face)}
                    </span>
                  </div>
                )}
              </>
            ) : !alarm.cropImageRelPath ? (
              <div className="flex h-full w-full items-center justify-center font-mono text-sm text-slate-500">
                {t('modal.noImage')}
              </div>
            ) : null}
          </div>

          {/* 底部详情与操作卡片 */}
          <div className="flex flex-wrap items-center justify-between gap-4 rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4">
            <div className="flex items-center gap-4">
              {alarm.cropImageRelPath && (
                <button
                  type="button"
                  onClick={onSelectCrop}
                  className="h-16 w-16 shrink-0 overflow-hidden rounded-xl border border-white/30 bg-black/80 shadow-xs transition-transform hover:scale-105"
                  title={t('modal.cropImage')}
                >
                  <img
                    src={evidenceApi.getImageUrl(alarm.cropImageRelPath)}
                    alt="Crop"
                    className="h-full w-full object-cover"
                  />
                </button>
              )}
              <div className="space-y-1 text-xs">
                <div className="flex items-center gap-2 font-semibold text-[var(--text-primary)]">
                  <span>{t('modal.cropImage')}</span>
                  <span
                    className={`py-0.2 rounded px-1.5 text-[9px] font-bold uppercase ${
                      alarm.severity === 'critical'
                        ? 'bg-rose-500/20 text-rose-500'
                        : 'bg-amber-500/20 text-amber-500'
                    }`}
                  >
                    {alarm.severity || 'WARNING'}
                  </span>
                </div>
                <div className="font-mono text-[11px] text-[var(--text-muted)]">
                  {t('modal.channel')}: {cameraName || alarm.cameraId} · {t('modal.trackId')}: #
                  {alarm.trackId}
                </div>
                <div className="font-mono text-[11px] text-[var(--text-muted)]">
                  {t('modal.time')}: {formatTimestamp(alarm.occurredAt)}
                </div>
              </div>
            </div>

            <div className="flex items-center gap-3">
              {faceConfidence !== undefined && (
                <span className="rounded-xl border border-purple-500/30 bg-purple-500/10 px-3 py-1.5 font-mono text-xs font-semibold text-purple-400">
                  {t('modal.faceConfidence')}: {faceConfidence.toFixed(2)}
                </span>
              )}
              <button
                type="button"
                onClick={onToggleStatus}
                className={`flex items-center gap-1.5 rounded-xl px-4 py-2 text-xs font-semibold transition-all ${
                  isProcessed
                    ? 'border border-emerald-500/30 bg-emerald-500/15 text-emerald-500 hover:bg-emerald-500/25'
                    : 'bg-rose-500 text-white shadow-xs hover:opacity-90'
                }`}
              >
                {isProcessed ? (
                  <>
                    <CheckCircle2 className="h-4 w-4" />
                    <span>{t('card.processed')}</span>
                  </>
                ) : (
                  <>
                    <Check className="h-4 w-4" />
                    <span>{t('card.markProcessed')}</span>
                  </>
                )}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
