import React from 'react'
import { AlertCircle, Check, CheckCircle2, Users, X, XCircle } from 'lucide-react'
import { evidenceApi } from '../../../lib/api'
import type { RecognitionRecord } from '../../../types'
import { formatTimestamp } from '../utils'

export interface RecognitionCardItemProps {
  recognition: RecognitionRecord
  onOpenReview: (rec: RecognitionRecord) => void
  onQuickReview: (rec: RecognitionRecord, status: 'confirmed' | 'rejected') => void
  t: (key: string) => string
}

export function RecognitionCardItem({
  recognition,
  onOpenReview,
  onQuickReview,
  t,
}: RecognitionCardItemProps): React.ReactElement {
  const isPending = recognition.status === 'pending_review'
  const isConfirmed = recognition.status === 'confirmed'
  const isRejected = recognition.status === 'rejected'
  const candidates = recognition.candidates || []
  const registeredPhotoRel = recognition.registeredPhotoPath || candidates[0]?.photoRelPath || ''
  const [photoLoadError, setPhotoLoadError] = React.useState(false)

  const simPct = ((recognition.similarity ?? 0) * 100).toFixed(0)

  let statusTextColor = 'text-emerald-500'
  let statusBadgeClass = 'border-emerald-500/40 bg-emerald-500/15 text-emerald-500'
  if (isPending) {
    statusTextColor = 'text-amber-500'
    statusBadgeClass = 'border-amber-500/40 bg-amber-500/15 text-amber-500'
  } else if (isRejected) {
    statusTextColor = 'text-rose-400'
    statusBadgeClass = 'border-rose-500/40 bg-rose-500/15 text-rose-400'
  }

  return (
    <div
      className={`flex flex-col justify-between space-y-3 rounded-2xl border bg-[var(--bg-surface)] p-3.5 shadow-sm transition-all hover:shadow-md ${
        isPending
          ? 'border-amber-500/40 bg-amber-500/5'
          : isRejected
            ? 'border-rose-500/30 opacity-75'
            : 'border-[var(--border)] hover:border-emerald-500/40'
      }`}
    >
      {/* 头部：状态标签与相似度 */}
      <div className="flex items-center justify-between">
        <div>
          {isConfirmed && (
            <span className="inline-flex items-center gap-1 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 font-mono text-[10px] font-semibold text-emerald-500">
              <CheckCircle2 className="h-3 w-3" />
              {t('card.statusConfirmed')}
            </span>
          )}
          {isPending && (
            <span className="inline-flex items-center gap-1 rounded-full border border-amber-500/30 bg-amber-500/15 px-2 py-0.5 font-mono text-[10px] font-semibold text-amber-500">
              <AlertCircle className="h-3 w-3" />
              {t('card.statusPendingReview')}
            </span>
          )}
          {isRejected && (
            <span className="inline-flex items-center gap-1 rounded-full border border-rose-500/30 bg-rose-500/10 px-2 py-0.5 font-mono text-[10px] font-semibold text-rose-400">
              <XCircle className="h-3 w-3" />
              {t('card.statusRejected')}
            </span>
          )}
        </div>

        <span className="font-mono text-[10px] text-[var(--text-muted)]">
          {recognition.cameraId}
        </span>
      </div>

      {/* 图像对比区 */}
      <div className="flex items-center justify-between gap-3">
        {/* 现场抓拍特写 */}
        <div className="flex flex-1 flex-col items-center gap-1.5">
          <div className="aspect-square w-full overflow-hidden rounded-xl border border-[var(--border)] bg-black/90 shadow-xs">
            {recognition.fieldCropPath ? (
              <img
                src={evidenceApi.getImageUrl(recognition.fieldCropPath)}
                alt={t('card.siteCrop')}
                className="h-full w-full object-cover"
              />
            ) : (
              <div className="flex h-full items-center justify-center text-[10px] text-slate-500">
                {t('card.noImage')}
              </div>
            )}
          </div>
          <span className="text-[10px] text-[var(--text-muted)]">{t('card.siteCrop')}</span>
        </div>

        {/* 相似度分值徽标 */}
        <div className="flex flex-col items-center gap-1 px-1">
          <span className={`font-mono text-[9px] font-bold uppercase ${statusTextColor}`}>
            {t('card.match')}
          </span>
          <div
            className={`flex h-11 w-11 items-center justify-center rounded-full border font-mono text-xs font-bold shadow-xs ${statusBadgeClass}`}
          >
            {simPct}%
          </div>
          <span className="text-[9px] text-[var(--text-muted)]">{t('card.similarity')}</span>
        </div>

        {/* 底库登记照片 */}
        <div className="flex flex-1 flex-col items-center gap-1.5">
          <div className="aspect-square w-full overflow-hidden rounded-xl border border-[var(--border)] bg-black/90 shadow-xs">
            {registeredPhotoRel && !photoLoadError ? (
              <img
                src={evidenceApi.getImageUrl(registeredPhotoRel)}
                alt={t('card.registeredPhoto')}
                className="h-full w-full object-cover"
                onError={() => setPhotoLoadError(true)}
              />
            ) : (
              <div className="flex h-full items-center justify-center text-[10px] text-slate-500">
                {t('card.noImage')}
              </div>
            )}
          </div>
          <span className="text-[10px] text-[var(--text-muted)]">{t('card.registeredPhoto')}</span>
        </div>
      </div>

      {/* 人员身份与时间 */}
      <div className="border-t border-[var(--border)] pt-2 text-xs">
        <div className="flex items-center justify-between">
          <div>
            <span className="font-semibold text-[var(--text-primary)]">
              {recognition.subjectName}
            </span>
            <span className="ml-1.5 font-mono text-[10px] text-[var(--text-muted)]">
              ID: {recognition.subjectId}
            </span>
          </div>
          <span className="font-mono text-[10px] text-[var(--text-muted)]">
            {formatTimestamp(recognition.recognizedAt)}
          </span>
        </div>

        {/* 候选人对比与核验按钮 */}
        <div className="mt-2.5 flex items-center justify-between gap-1.5 pt-1">
          {candidates.length > 0 && (
            <button
              onClick={() => onOpenReview(recognition)}
              className="flex items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-base)] px-2 py-1 font-mono text-[10px] font-medium text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)]"
            >
              <Users className="h-3 w-3" />
              <span>
                Top-{candidates.length} {t('card.candidateList')}
              </span>
            </button>
          )}

          {isPending && (
            <div className="flex items-center gap-1.5">
              <button
                onClick={() => onQuickReview(recognition, 'confirmed')}
                className="flex items-center gap-1 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-[10px] font-medium text-emerald-500 transition-colors hover:bg-emerald-500 hover:text-white"
                title={t('card.passTop1')}
              >
                <Check className="h-3 w-3" />
                <span>{t('card.passTop1')}</span>
              </button>
              <button
                onClick={() => onQuickReview(recognition, 'rejected')}
                className="flex items-center gap-1 rounded-lg border border-rose-500/30 bg-rose-500/10 px-2 py-1 text-[10px] font-medium text-rose-500 transition-colors hover:bg-rose-500 hover:text-white"
                title={t('card.reject')}
              >
                <X className="h-3 w-3" />
                <span>{t('card.reject')}</span>
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
