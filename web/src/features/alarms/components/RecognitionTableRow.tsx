import React, { useState } from 'react'
import {
  AlertCircle,
  Camera,
  Check,
  CheckCircle2,
  Image as ImageIcon,
  User,
  Users,
  X,
  XCircle,
  ZoomIn,
} from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { evidenceApi } from '@/lib/api'
import type { RecognitionRecord } from '@/types'
import { formatCosineSimilarityPercent, getCosineSimilarityLevel } from '@/lib/similarity'
import { formatTimestamp } from '../utils'
import { ImagePreviewModal } from './ImagePreviewModal'
import { RECOGNITION_STATUS_STYLES, SIMILARITY_STYLES } from '../statusStyles'

export interface RecognitionTableRowProps {
  recognition: RecognitionRecord
  cameraName?: string
  onOpenReview: (rec: RecognitionRecord) => void
  onQuickReview: (rec: RecognitionRecord, status: 'confirmed' | 'rejected') => void
  t: (key: string, options?: Record<string, unknown>) => string
}

const STATUS_CONFIG = {
  confirmed: {
    ...RECOGNITION_STATUS_STYLES.confirmed,
    Icon: CheckCircle2,
    labelKey: 'card.statusConfirmed',
  },
  pending_review: {
    ...RECOGNITION_STATUS_STYLES.pending_review,
    Icon: AlertCircle,
    labelKey: 'card.statusPendingReview',
  },
  rejected: {
    ...RECOGNITION_STATUS_STYLES.rejected,
    Icon: XCircle,
    labelKey: 'card.statusRejected',
  },
} as const

interface PureThumbProps {
  src?: string | null
  alt: string
  title?: string
  clickToZoomText?: string
  onZoom?: () => void
}

/**
 * 纯净头像缩略图 (内部零遮挡，100% 呈现人脸五官细节)
 */
function PureThumb({
  src,
  alt,
  title,
  clickToZoomText,
  onZoom,
}: PureThumbProps): React.ReactElement {
  const [hasError, setHasError] = useState(false)

  if (!src || hasError) {
    return (
      <div
        className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-[var(--border)] bg-black/60 text-slate-500"
        title={title || alt}
      >
        <User className="h-4 w-4 opacity-40" />
      </div>
    )
  }

  const tooltip = title || (clickToZoomText ? `${alt} (${clickToZoomText})` : alt)

  return (
    <div
      onClick={(e) => {
        e.stopPropagation()
        onZoom?.()
      }}
      className="group/thumb relative h-11 w-11 shrink-0 cursor-pointer overflow-hidden rounded-xl border border-[var(--border)] bg-black shadow-xs transition-all hover:border-[var(--accent)] hover:shadow-md active:scale-95"
      title={tooltip}
    >
      <img
        src={evidenceApi.getImageUrl(src)}
        alt={alt}
        loading="lazy"
        decoding="async"
        onError={() => setHasError(true)}
        className="h-full w-full object-cover transition-transform duration-200 group-hover/thumb:scale-110"
      />
      {/* 仅在鼠标 hover 时显示半透明居中放大镜，平时图片全貌绝对零遮挡 */}
      <div className="backdrop-blur-2xs absolute inset-0 flex items-center justify-center bg-black/40 opacity-0 transition-opacity duration-200 group-hover/thumb:opacity-100">
        <ZoomIn className="h-3.5 w-3.5 text-white" />
      </div>
    </div>
  )
}

export const RecognitionTableRow = React.memo(function RecognitionTableRow({
  recognition,
  cameraName,
  onOpenReview,
  onQuickReview,
  t,
}: RecognitionTableRowProps): React.ReactElement {
  const isPending = recognition.status === 'pending_review'
  const candidates = recognition.candidates || []
  const registeredPhotoRel = recognition.registeredPhotoPath || candidates[0]?.photoRelPath || ''
  const fieldImagePath = recognition.fieldImagePath
  const fieldCropPath = recognition.fieldCropPath || fieldImagePath || ''

  const [previewModal, setPreviewModal] = useState<{
    src: string
    title?: string
    subtitle?: string
    bboxJson?: string | null
  } | null>(null)

  const statusConfig =
    STATUS_CONFIG[recognition.status as keyof typeof STATUS_CONFIG] ?? STATUS_CONFIG.confirmed
  const simPct = formatCosineSimilarityPercent(recognition.similarity, 0)
  const simLevel = getCosineSimilarityLevel(recognition.similarity)

  const simColorClass = SIMILARITY_STYLES[simLevel]

  return (
    <>
      <tr
        tabIndex={0}
        role="row"
        onClick={() => onOpenReview(recognition)}
        onKeyDown={(e) => {
          if (e.target !== e.currentTarget) return
          if (e.key === 'Enter') {
            e.preventDefault()
            onOpenReview(recognition)
          } else if (e.key === ' ') {
            e.preventDefault()
            if (isPending) {
              onQuickReview(recognition, 'confirmed')
            } else {
              onQuickReview(
                recognition,
                recognition.status === 'confirmed' ? 'rejected' : 'confirmed',
              )
            }
          }
        }}
        className="group cursor-pointer transition-colors hover:bg-[var(--accent-soft)]/20 focus-visible:bg-[var(--accent-soft)]/30 focus-visible:outline-none"
      >
        {/* 状态 Pill */}
        <td className="px-3.5 py-3 whitespace-nowrap">
          <span
            className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 font-mono text-[10px] font-semibold tracking-wide shadow-2xs backdrop-blur-xs ${statusConfig.chipClass}`}
          >
            <span className={`h-1.5 w-1.5 rounded-full ${statusConfig.dotClass}`} />
            {t(statusConfig.labelKey)}
          </span>
        </td>

        {/* 现场抓拍头像 (独立列，零遮挡) */}
        <td className="px-3.5 py-3 whitespace-nowrap">
          <PureThumb
            src={fieldCropPath}
            alt={t('card.siteCrop')}
            clickToZoomText={t('modal.clickToZoom')}
            onZoom={() => {
              if (fieldCropPath) {
                setPreviewModal({
                  src: evidenceApi.getImageUrl(fieldCropPath),
                  title: `${t('card.siteCrop')} · ${recognition.subjectName || recognition.cameraId}`,
                  subtitle: `${cameraName || recognition.cameraId} · ${formatTimestamp(recognition.recognizedAt)}`,
                })
              }
            }}
          />
        </td>

        {/* 相似度得分 */}
        <td className="px-3.5 py-3 whitespace-nowrap">
          <span
            className={`inline-flex items-center justify-center rounded-xl border px-2.5 py-1 font-mono text-xs font-bold tracking-tight backdrop-blur-xs ${simColorClass}`}
          >
            {simPct}
          </span>
        </td>

        {/* 底库样本头像 (独立列，零遮挡) */}
        <td className="px-3.5 py-3 whitespace-nowrap">
          <PureThumb
            src={registeredPhotoRel}
            alt={t('card.registeredPhoto')}
            clickToZoomText={t('modal.clickToZoom')}
            onZoom={() => {
              if (registeredPhotoRel) {
                setPreviewModal({
                  src: evidenceApi.getImageUrl(registeredPhotoRel),
                  title: `${t('card.registeredPhoto')}: ${recognition.subjectName || t('card.unknownSubject')}`,
                  subtitle: `ID: ${recognition.subjectId || '-'}`,
                })
              }
            }}
          />
        </td>

        {/* 目标身份 */}
        <td className="px-3.5 py-3">
          <div className="flex flex-col">
            <span className="font-semibold tracking-tight text-[var(--text-primary)]">
              {recognition.subjectName || t('card.unknownSubject')}
            </span>
            <span className="font-mono text-[10px] text-[var(--text-muted)]">
              ID: {recognition.subjectId || '-'}
            </span>
          </div>
        </td>

        {/* 摄像头通道 */}
        <td className="px-3.5 py-3 whitespace-nowrap">
          <div
            className="flex w-fit items-center gap-1.5 rounded-full border border-[var(--border)]/60 bg-[var(--bg-secondary)]/40 px-2.5 py-1 backdrop-blur-xs"
            title={cameraName || recognition.cameraId}
          >
            <Camera className="h-3 w-3 text-[var(--text-muted)]" />
            <span className="max-w-[130px] truncate text-[11px] font-medium text-[var(--text-secondary)]">
              {cameraName || recognition.cameraId}
            </span>
          </div>
        </td>

        {/* 抓拍时间 */}
        <td className="px-3.5 py-3 font-mono text-[11px] whitespace-nowrap text-[var(--text-muted)] tabular-nums">
          {formatTimestamp(recognition.recognizedAt)}
        </td>

        {/* 现场全景入口 */}
        <td className="px-3.5 py-3 whitespace-nowrap">
          {fieldImagePath ? (
            <button
              type="button"
              onClick={() => {
                setPreviewModal({
                  src: evidenceApi.getImageUrl(fieldImagePath),
                  bboxJson: recognition.fieldBboxJson,
                  title: `${t('card.sitePanorama')} · ${recognition.subjectName || recognition.cameraId}`,
                  subtitle: `${cameraName || recognition.cameraId} · ${formatTimestamp(recognition.recognizedAt)}`,
                })
              }}
              className="flex items-center gap-1 rounded-full border border-[var(--border)]/80 bg-[var(--bg-surface)]/80 px-2.5 py-1 text-[10px] font-medium text-[var(--text-secondary)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] active:scale-95"
              title={t('card.viewPanorama')}
            >
              <ImageIcon className="h-3 w-3" />
              <span>{t('card.viewPanorama')}</span>
            </button>
          ) : (
            <span className="text-[11px] text-[var(--text-muted)] opacity-40">-</span>
          )}
        </td>

        {/* 操作 */}
        <td className="px-3.5 py-3 text-right whitespace-nowrap">
          <div className="flex items-center justify-end gap-1.5">
            {candidates.length > 0 && (
              <button
                type="button"
                onClick={() => onOpenReview(recognition)}
                className="flex items-center gap-1 rounded-xl border border-[var(--border)]/80 bg-[var(--bg-surface)]/80 px-2.5 py-1 font-mono text-[10px] font-medium text-[var(--text-secondary)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] active:scale-95"
              >
                <Users className="h-3 w-3" />
                <span>Top-{candidates.length}</span>
              </button>
            )}

            {isPending ? (
              <>
                <motion.button
                  type="button"
                  whileTap={{ scale: 0.96 }}
                  onClick={() => onQuickReview(recognition, 'confirmed')}
                  className="flex items-center gap-1 rounded-xl border border-[var(--status-success-border)] bg-[var(--status-success-soft)] px-2.5 py-1 text-[11px] font-semibold text-[var(--status-success)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--status-success)] hover:bg-[var(--status-success)] hover:text-white"
                  title={t('card.passTop1')}
                >
                  <Check className="h-3 w-3 stroke-[2.5]" />
                  <span>{t('card.passTop1')}</span>
                </motion.button>
                <motion.button
                  type="button"
                  whileTap={{ scale: 0.96 }}
                  onClick={() => onQuickReview(recognition, 'rejected')}
                  className="flex items-center gap-1 rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] px-2.5 py-1 text-[11px] font-semibold text-[var(--status-danger)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--status-danger)] hover:bg-[var(--status-danger-solid)] hover:text-white"
                  title={t('card.reject')}
                >
                  <X className="h-3 w-3 stroke-[2.5]" />
                  <span>{t('card.reject')}</span>
                </motion.button>
              </>
            ) : (
              <button
                type="button"
                onClick={() =>
                  onQuickReview(
                    recognition,
                    recognition.status === 'confirmed' ? 'rejected' : 'confirmed',
                  )
                }
                className="rounded-xl border border-[var(--border)]/80 bg-[var(--bg-surface)]/80 px-2.5 py-1 text-[10px] font-medium text-[var(--text-muted)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--accent)] hover:text-[var(--accent)] active:scale-95"
              >
                {recognition.status === 'confirmed' ? t('card.reject') : t('card.passTop1')}
              </button>
            )}
          </div>
        </td>
      </tr>

      {/* 弹窗全屏预览 */}
      <AnimatePresence>
        {previewModal && (
          <ImagePreviewModal
            src={previewModal.src}
            title={previewModal.title}
            subtitle={previewModal.subtitle}
            bboxJson={previewModal.bboxJson}
            onClose={() => setPreviewModal(null)}
          />
        )}
      </AnimatePresence>
    </>
  )
})
