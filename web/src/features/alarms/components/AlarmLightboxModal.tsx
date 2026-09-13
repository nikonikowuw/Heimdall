import React, { useRef, useState } from 'react'
import { Check, CheckCircle2, Download, ShieldAlert, X } from 'lucide-react'
import { useDismissStack } from '../../../hooks/use-dismiss-stack'
import { evidenceApi } from '../../../lib/api'
import type { AlarmRecord } from '../../../types'
import {
  calculateFittedImageRect,
  type FittedImageRect,
  formatTimestamp,
  getRuleTypeLabel,
  parseBBoxCoords,
} from '../utils'

export interface AlarmLightboxModalProps {
  alarm: AlarmRecord
  cameraName?: string
  onClose: () => void
  onToggleStatus: () => void
  onSelectCrop?: () => void
  t: (key: string) => string
}

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
  const [imgRect, setImgRect] = useState<FittedImageRect | null>(null)

  const bbox = parseBBoxCoords(alarm.bboxJson)

  // 支持 ESC 浮层栈快速退出
  useDismissStack(true, onClose)

  const handleImageLoad = (e: React.SyntheticEvent<HTMLImageElement>) => {
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
            {alarm.imageRelPath ? (
              <>
                <img
                  src={evidenceApi.getImageUrl(alarm.imageRelPath)}
                  alt="Full Frame"
                  className="h-full w-full object-contain"
                  onLoad={handleImageLoad}
                />
                {bbox && imgRect && (
                  <div
                    className="pointer-events-none absolute border-2 border-rose-500 bg-rose-500/15 shadow-[0_0_15px_rgba(244,63,94,0.6)] transition-all"
                    style={{
                      left: `${imgRect.x + bbox.x1 * imgRect.width}px`,
                      top: `${imgRect.y + bbox.y1 * imgRect.height}px`,
                      width: `${Math.max(6, (bbox.x2 - bbox.x1) * imgRect.width)}px`,
                      height: `${Math.max(6, (bbox.y2 - bbox.y1) * imgRect.height)}px`,
                    }}
                  >
                    <span className="absolute -top-6 left-0 rounded bg-rose-600 px-2 py-0.5 font-mono text-[10px] font-bold whitespace-nowrap text-white shadow-xs">
                      #{alarm.trackId} {alarm.targetLabel} (
                      {((alarm.confidence ?? 0) * 100).toFixed(0)}%)
                    </span>
                  </div>
                )}
              </>
            ) : (
              <div className="flex h-full w-full items-center justify-center font-mono text-sm text-slate-500">
                {t('modal.noImage')}
              </div>
            )}
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
