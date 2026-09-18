import React, { useRef } from 'react'
import { Download, ShieldAlert, X } from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { useDismissStack } from '../../../hooks/use-dismiss-stack'
import { evidenceApi } from '../../../lib/api'
import { motionTokens } from '@/lib/motionTokens'
import type { AlarmRecord } from '../../../types'
import { useImageZoomPan } from '../hooks/useImageZoomPan'
import { ZoomControls } from './ZoomControls'
import { formatTimestamp, getRuleTypeLabel } from '../utils'

export interface CropLightboxModalProps {
  alarm: AlarmRecord
  onClose: () => void
  t: (key: string, options?: Record<string, unknown>) => string
}

export function CropLightboxModal({
  alarm,
  onClose,
  t,
}: CropLightboxModalProps): React.ReactElement {
  // 嵌套层级更高（特写弹窗基于告警大图弹出），设置 priority: 10 保证后入先出响应
  useDismissStack(true, onClose, { priority: 10 })
  const shouldReduce = useReducedMotion()

  const containerRef = useRef<HTMLDivElement>(null)
  const { zoom, zoomIn, zoomOut, resetZoom, dragProps, containerCursorClass, transformStyle } =
    useImageZoomPan(containerRef)

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{
        duration: shouldReduce ? 0.1 : motionTokens.duration.fast,
        ease: motionTokens.easing.smooth,
      }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4 backdrop-blur-sm select-none"
      onClick={onClose}
    >
      <motion.div
        initial={{ opacity: 0, scale: shouldReduce ? 1 : 0.96, y: shouldReduce ? 0 : 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: shouldReduce ? 1 : 0.96, y: shouldReduce ? 0 : 8 }}
        transition={{
          duration: shouldReduce ? 0.1 : motionTokens.duration.normal,
          ease: motionTokens.easing.smooth,
        }}
        className="relative flex max-h-[90vh] w-full max-w-lg flex-col overflow-hidden rounded-3xl border border-[var(--border)] bg-[var(--bg-surface)] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-[var(--border)] bg-[var(--bg-secondary)]/50 px-6 py-3.5">
          <div className="flex items-center gap-2">
            <ShieldAlert className="h-5 w-5 text-rose-500" />
            <h3 className="text-sm font-semibold text-[var(--text-primary)]">
              {t('modal.cropImage')} · {alarm.targetLabel}
            </h3>
          </div>
          <div className="flex items-center gap-2">
            {alarm.cropImageRelPath && (
              <a
                href={evidenceApi.getImageUrl(alarm.cropImageRelPath)}
                download={`crop_${alarm.eventId}.jpg`}
                target="_blank"
                rel="noreferrer"
                className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1 text-xs text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                aria-label={t('modal.download')}
              >
                <Download className="h-3.5 w-3.5" />
              </a>
            )}
            <button
              type="button"
              onClick={onClose}
              className="rounded-xl p-1.5 text-[var(--text-muted)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--accent)]"
              aria-label="Close modal"
            >
              <X className="h-5 w-5" />
            </button>
          </div>
        </div>

        <div className="flex-1 space-y-4 overflow-auto p-4 sm:p-6">
          <div
            ref={containerRef}
            {...dragProps}
            className={`relative flex aspect-square w-full items-center justify-center overflow-hidden rounded-2xl border border-[var(--border)] bg-black shadow-md select-none ${containerCursorClass}`}
          >
            {/* 浮动缩放控制条 */}
            {alarm.cropImageRelPath && (
              <div className="absolute top-3 right-3 z-30">
                <ZoomControls
                  zoom={zoom}
                  onZoomIn={zoomIn}
                  onZoomOut={zoomOut}
                  onResetZoom={resetZoom}
                  t={t}
                  variant="dark"
                />
              </div>
            )}

            {alarm.cropImageRelPath ? (
              <div
                className="relative flex h-full w-full items-center justify-center will-change-transform"
                style={transformStyle}
              >
                <img
                  src={evidenceApi.getImageUrl(alarm.cropImageRelPath)}
                  alt="Crop Preview"
                  draggable={false}
                  className="max-h-full max-w-full object-contain select-none"
                />
              </div>
            ) : (
              <div className="flex h-full items-center justify-center font-mono text-sm text-[var(--text-muted)]">
                {t('modal.noImage')}
              </div>
            )}
          </div>

          <div className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/30 p-3 text-xs">
            <div className="flex items-center gap-2">
              <span className="font-semibold text-[var(--text-primary)]">{alarm.targetLabel}</span>
              <span className="font-mono font-bold text-[var(--accent)]">#{alarm.trackId}</span>
              <span className="text-[var(--text-muted)]">
                ({((alarm.confidence ?? 0) * 100).toFixed(0)}%)
              </span>
            </div>
            <span className="font-mono text-[var(--text-muted)]">
              {formatTimestamp(alarm.occurredAt)}
            </span>
          </div>

          <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/30 p-3 text-xs text-[var(--text-secondary)]">
            <span className="text-[var(--text-muted)]">{t('columns.ruleType')}:</span>{' '}
            <span>{getRuleTypeLabel(alarm.ruleType, t)}</span>
          </div>
        </div>

        <div className="flex items-center justify-between border-t border-[var(--border)] bg-[var(--bg-secondary)]/20 px-6 py-3 font-mono text-[10px] text-[var(--text-muted)]">
          <span>{t('modal.escHint')}</span>
          <span>{t('modal.zoomHint')}</span>
        </div>
      </motion.div>
    </motion.div>
  )
}
