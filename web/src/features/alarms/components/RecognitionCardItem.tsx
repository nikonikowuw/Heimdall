import React, { useState } from 'react'
import { AlertCircle, Check, CheckCircle2, Users, X, XCircle, ZoomIn } from 'lucide-react'
import { evidenceApi } from '../../../lib/api'
import type { RecognitionRecord } from '../../../types'
import { formatCosineSimilarityPercent } from '@/lib/similarity'
import { formatTimestamp } from '../utils'
import { ImagePreviewModal } from './ImagePreviewModal'
import { RecognitionEvidencePreview } from './RecognitionEvidencePreview'

export interface RecognitionCardItemProps {
  recognition: RecognitionRecord
  cameraName?: string
  onOpenReview: (rec: RecognitionRecord) => void
  onQuickReview: (rec: RecognitionRecord, status: 'confirmed' | 'rejected') => void
  t: (key: string) => string
}

const STATUS_CONFIG = {
  confirmed: {
    textColor: 'text-emerald-500',
    badgeClass: 'border-emerald-500/40 bg-emerald-500/15 text-emerald-500',
    chipClass: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-500',
    cardBorder: 'border-[var(--border)] hover:border-emerald-500/40',
    Icon: CheckCircle2,
    labelKey: 'card.statusConfirmed',
  },
  pending_review: {
    textColor: 'text-amber-500',
    badgeClass: 'border-amber-500/40 bg-amber-500/15 text-amber-500',
    chipClass: 'border-amber-500/30 bg-amber-500/15 text-amber-500',
    cardBorder: 'border-amber-500/40 bg-amber-500/5',
    Icon: AlertCircle,
    labelKey: 'card.statusPendingReview',
  },
  rejected: {
    textColor: 'text-rose-400',
    badgeClass: 'border-rose-500/40 bg-rose-500/15 text-rose-400',
    chipClass: 'border-rose-500/30 bg-rose-500/10 text-rose-400',
    cardBorder: 'border-rose-500/30 opacity-75',
    Icon: XCircle,
    labelKey: 'card.statusRejected',
  },
} as const

export function RecognitionCardItem({
  recognition,
  cameraName,
  onOpenReview,
  onQuickReview,
  t,
}: RecognitionCardItemProps): React.ReactElement {
  const isPending = recognition.status === 'pending_review'
  const candidates = recognition.candidates || []
  const registeredPhotoRel = recognition.registeredPhotoPath || candidates[0]?.photoRelPath || ''
  const fieldImagePath = recognition.fieldImagePath
  const [photoLoadError, setPhotoLoadError] = useState(false)
  const [previewModal, setPreviewModal] = useState<{
    src: string
    title?: string
    subtitle?: string
    bboxJson?: string | null
  } | null>(null)

  const statusConfig =
    STATUS_CONFIG[recognition.status as keyof typeof STATUS_CONFIG] ?? STATUS_CONFIG.confirmed
  const StatusIcon = statusConfig.Icon
  const simPct = formatCosineSimilarityPercent(recognition.similarity, 0)

  return (
    <div
      className={`flex flex-col justify-between space-y-3 rounded-2xl border bg-[var(--bg-surface)] p-3.5 shadow-sm transition-all hover:shadow-md ${statusConfig.cardBorder}`}
    >
      {/* 头部：状态标签与相机名称 */}
      <div className="flex items-center justify-between">
        <span
          className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 font-mono text-[10px] font-semibold ${statusConfig.chipClass}`}
        >
          <StatusIcon className="h-3 w-3" />
          {t(statusConfig.labelKey)}
        </span>

        <span
          className="max-w-[130px] truncate text-[11px] font-medium text-[var(--text-secondary)]"
          title={cameraName || recognition.cameraId}
        >
          {cameraName || recognition.cameraId}
        </span>
      </div>

      {/* 现场全景上下文 */}
      {fieldImagePath && (
        <RecognitionEvidencePreview
          src={evidenceApi.getImageUrl(fieldImagePath)}
          bboxJson={recognition.fieldBboxJson}
          alt={t('card.sitePanorama')}
          label={t('card.sitePanorama')}
          faceLabel={t('card.face')}
          className="aspect-video w-full"
          title={t('card.viewHd')}
          noImageText={t('card.noImage')}
          onPreview={() => {
            setPreviewModal({
              src: evidenceApi.getImageUrl(fieldImagePath),
              bboxJson: recognition.fieldBboxJson,
              title: `${t('card.sitePanorama')} · ${recognition.subjectName || recognition.cameraId}`,
              subtitle: `${cameraName || recognition.cameraId} · ${formatTimestamp(recognition.recognizedAt)}`,
            })
          }}
        />
      )}

      {/* 人脸特写与底库样本 */}
      <div className="flex items-center justify-between gap-3">
        {/* 现场抓拍特写 */}
        <div className="flex flex-1 flex-col items-center gap-1.5">
          <div
            onClick={(e) => {
              e.stopPropagation()
              if (recognition.fieldCropPath) {
                setPreviewModal({
                  src: evidenceApi.getImageUrl(recognition.fieldCropPath),
                  title: `${t('card.siteCrop')} · ${recognition.subjectName || recognition.cameraId}`,
                  subtitle: `${cameraName || recognition.cameraId} · ${formatTimestamp(recognition.recognizedAt)}`,
                })
              }
            }}
            className={`group/crop relative aspect-square w-full overflow-hidden rounded-xl border border-[var(--border)] bg-black/90 shadow-xs ${
              recognition.fieldCropPath
                ? 'cursor-pointer hover:border-[var(--accent)] hover:shadow-md'
                : ''
            }`}
            title={recognition.fieldCropPath ? t('card.viewHd') : undefined}
          >
            {recognition.fieldCropPath ? (
              <>
                <img
                  src={evidenceApi.getImageUrl(recognition.fieldCropPath)}
                  alt={t('card.siteCrop')}
                  className="h-full w-full object-cover transition-transform duration-300 group-hover/crop:scale-105"
                />
                <div className="backdrop-blur-2xs absolute inset-0 flex items-center justify-center bg-black/40 opacity-0 transition-opacity duration-200 group-hover/crop:opacity-100">
                  <span className="flex items-center gap-1 rounded-lg bg-black/60 px-2 py-1 text-[10px] font-medium text-white shadow-xs">
                    <ZoomIn className="h-3 w-3" />
                    <span>{t('card.viewHd')}</span>
                  </span>
                </div>
              </>
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
          <span className={`font-mono text-[9px] font-bold uppercase ${statusConfig.textColor}`}>
            {t('card.match')}
          </span>
          <div
            className={`flex h-11 w-11 items-center justify-center rounded-full border font-mono text-xs font-bold shadow-xs ${statusConfig.badgeClass}`}
          >
            {simPct}
          </div>
          <span className="text-[9px] text-[var(--text-muted)]">{t('card.similarity')}</span>
        </div>

        {/* 底库登记照片 */}
        <div className="flex flex-1 flex-col items-center gap-1.5">
          <div
            onClick={(e) => {
              e.stopPropagation()
              if (registeredPhotoRel && !photoLoadError) {
                setPreviewModal({
                  src: evidenceApi.getImageUrl(registeredPhotoRel),
                  title: `${t('card.registeredPhoto')}: ${recognition.subjectName || '已登记人员'}`,
                  subtitle: `ID: ${recognition.subjectId || '-'}`,
                })
              }
            }}
            className={`group/reg relative aspect-square w-full overflow-hidden rounded-xl border border-[var(--border)] bg-black/90 shadow-xs ${
              registeredPhotoRel && !photoLoadError
                ? 'cursor-pointer hover:border-[var(--accent)] hover:shadow-md'
                : ''
            }`}
            title={registeredPhotoRel && !photoLoadError ? t('card.viewHd') : undefined}
          >
            {registeredPhotoRel && !photoLoadError ? (
              <>
                <img
                  src={evidenceApi.getImageUrl(registeredPhotoRel)}
                  alt={t('card.registeredPhoto')}
                  className="h-full w-full object-cover transition-transform duration-300 group-hover/reg:scale-105"
                  onError={() => setPhotoLoadError(true)}
                />
                <div className="backdrop-blur-2xs absolute inset-0 flex items-center justify-center bg-black/40 opacity-0 transition-opacity duration-200 group-hover/reg:opacity-100">
                  <span className="flex items-center gap-1 rounded-lg bg-black/60 px-2 py-1 text-[10px] font-medium text-white shadow-xs">
                    <ZoomIn className="h-3 w-3" />
                    <span>{t('card.viewHd')}</span>
                  </span>
                </div>
              </>
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

      {/* 高清图片大图全屏预览灯箱 */}
      {previewModal && (
        <ImagePreviewModal
          src={previewModal.src}
          title={previewModal.title}
          subtitle={previewModal.subtitle}
          bboxJson={previewModal.bboxJson}
          onClose={() => setPreviewModal(null)}
        />
      )}
    </div>
  )
}
