import React from 'react'
import { Download, ShieldAlert, X } from 'lucide-react'
import { useDismissStack } from '../../../hooks/use-dismiss-stack'
import { evidenceApi } from '../../../lib/api'
import type { AlarmRecord } from '../../../types'
import { formatTimestamp, getRuleTypeLabel } from '../utils'

export interface CropLightboxModalProps {
  alarm: AlarmRecord
  onClose: () => void
  t: (key: string) => string
}

export function CropLightboxModal({
  alarm,
  onClose,
  t,
}: CropLightboxModalProps): React.ReactElement {
  // 嵌套层级更高（特写弹窗基于告警大图弹出），设置 priority: 10 保证后入先出响应
  useDismissStack(true, onClose, { priority: 10 })

  return (
    <div
      className="animate-in fade-in fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4 backdrop-blur-sm duration-150"
      onClick={onClose}
    >
      <div
        className="relative flex max-h-[90vh] w-full max-w-lg flex-col overflow-hidden rounded-3xl border border-[var(--border)] bg-[var(--bg-surface)] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-[var(--border)] bg-[var(--bg-secondary)]/50 px-6 py-4">
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
              >
                <Download className="h-3.5 w-3.5" />
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
          <div className="relative flex aspect-square w-full items-center justify-center overflow-hidden rounded-2xl border border-[var(--border)] bg-black shadow-md">
            {alarm.cropImageRelPath ? (
              <img
                src={evidenceApi.getImageUrl(alarm.cropImageRelPath)}
                alt="Target Crop"
                className="h-full w-full object-contain"
              />
            ) : (
              <div className="flex h-full w-full items-center justify-center font-mono text-xs text-slate-500">
                {t('card.noImage')}
              </div>
            )}
          </div>

          <div className="space-y-2 rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4 text-xs">
            <div className="flex items-center justify-between">
              <span className="font-semibold text-[var(--text-primary)]">
                {t('card.target')}: {alarm.targetLabel}
              </span>
              <span className="font-mono font-bold text-[var(--accent)]">
                {((alarm.confidence ?? 0) * 100).toFixed(0)}% {t('card.confidence')}
              </span>
            </div>
            <div className="flex items-center justify-between font-mono text-[11px] text-[var(--text-muted)]">
              <span>
                {t('modal.trackId')}: #{alarm.trackId}
              </span>
              <span>{getRuleTypeLabel(alarm.ruleType, t)}</span>
            </div>
            <div className="font-mono text-[11px] text-[var(--text-muted)]">
              {formatTimestamp(alarm.occurredAt)}
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
