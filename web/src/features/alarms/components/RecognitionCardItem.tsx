import React, { useState } from 'react'
import {
  AlertCircle,
  Camera,
  Check,
  CheckCircle2,
  Clock,
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

export interface RecognitionCardItemProps {
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

interface AvatarBoxProps {
  src?: string | null
  alt: string
  label?: string
  onZoom?: () => void
  viewHdText: string
  noImageText: string
}

function AvatarBox({
  src,
  alt,
  label,
  onZoom,
  viewHdText,
  noImageText,
}: AvatarBoxProps): React.ReactElement {
  const [isLoaded, setIsLoaded] = useState(false)
  const [hasError, setHasError] = useState(false)

  const isClickable = Boolean(src && !hasError && onZoom)

  return (
    <div className="flex w-full flex-col items-center gap-1.5">
      {label && (
        <span className="text-[10px] font-medium tracking-wider text-[var(--text-muted)]">
          {label}
        </span>
      )}
      <div
        onClick={isClickable ? onZoom : undefined}
        className={`group/avatar relative aspect-square w-full overflow-hidden rounded-2xl border border-[var(--border)] bg-black/90 shadow-inner transition-all duration-300 ${
          isClickable
            ? 'cursor-pointer hover:border-[var(--accent)] hover:shadow-md active:scale-98'
            : ''
        }`}
        title={isClickable ? viewHdText : undefined}
      >
        {/* 渐变加载骨架屏 */}
        {!isLoaded && !hasError && src && (
          <div className="absolute inset-0 animate-pulse bg-gradient-to-br from-slate-800 to-slate-900" />
        )}

        {src && !hasError ? (
          <>
            <img
              src={evidenceApi.getImageUrl(src)}
              alt={alt}
              loading="lazy"
              decoding="async"
              onLoad={() => setIsLoaded(true)}
              onError={() => setHasError(true)}
              className={`h-full w-full object-cover transition-all duration-300 group-hover/avatar:scale-105 ${
                isLoaded ? 'opacity-100' : 'opacity-0'
              }`}
            />
            {/* 悬浮查看大图毛玻璃遮罩 */}
            <div className="backdrop-blur-2xs absolute inset-0 flex items-center justify-center bg-black/40 opacity-0 transition-opacity duration-200 group-hover/avatar:opacity-100">
              <span className="flex items-center gap-1.5 rounded-xl border border-white/20 bg-black/70 px-2.5 py-1 text-[11px] font-medium text-white shadow-lg backdrop-blur-md">
                <ZoomIn className="h-3.5 w-3.5" />
                <span>{viewHdText}</span>
              </span>
            </div>
          </>
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-1.5 p-2 text-slate-500">
            <User className="h-7 w-7 opacity-30" />
            <span className="text-[10px] opacity-60">{noImageText}</span>
          </div>
        )}
      </div>
    </div>
  )
}

export const RecognitionCardItem = React.memo(function RecognitionCardItem({
  recognition,
  cameraName,
  onOpenReview,
  onQuickReview,
  t,
}: RecognitionCardItemProps): React.ReactElement {
  const isPending = recognition.status === 'pending_review'
  const candidates = recognition.candidates || []
  const registeredPhotoRel = recognition.registeredPhotoPath || candidates[0]?.photoRelPath || ''
  const fieldCropPath = recognition.fieldCropPath
  const fieldImagePath = recognition.fieldImagePath

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
    <div
      className={`group/card relative flex flex-col justify-between overflow-hidden rounded-3xl border border-[var(--border)]/70 bg-[var(--bg-surface)]/60 p-4 shadow-xs backdrop-blur-xl transition-all duration-300 hover:-translate-y-0.5 hover:shadow-xl sm:p-5 ${statusConfig.cardBorder}`}
    >
      {/* 顶部微妙光晕背景 */}
      <div
        className={`pointer-events-none absolute inset-x-0 top-0 h-24 bg-gradient-to-b ${statusConfig.ambientGlow} to-transparent opacity-60`}
      />
      {/* 顶部 1px 细微高光反射线 */}
      <div className="pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-white/20 to-transparent" />

      {/* 头部：状态发光 Pill 与通道来源胶囊 */}
      <div className="relative z-10 flex items-center justify-between gap-2">
        <span
          className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 font-mono text-[10px] font-semibold tracking-wide shadow-2xs backdrop-blur-xs ${statusConfig.chipClass}`}
        >
          <span className={`h-1.5 w-1.5 rounded-full ${statusConfig.dotClass}`} />
          {t(statusConfig.labelKey)}
        </span>

        <div
          className="flex shrink-0 items-center gap-1 rounded-full border border-[var(--border)]/60 bg-[var(--bg-secondary)]/40 px-2.5 py-0.5 shadow-2xs backdrop-blur-xs"
          title={cameraName || recognition.cameraId}
        >
          <Camera className="h-3 w-3 text-[var(--text-muted)]" />
          <span className="max-w-[130px] truncate text-[11px] font-medium text-[var(--text-secondary)]">
            {cameraName || recognition.cameraId}
          </span>
        </div>
      </div>

      {/* 核心对账舞台 (Frosted Glass Stage: 现场抓拍 VS 底库档案) */}
      <div className="relative z-10 my-3 rounded-2xl border border-[var(--border)]/60 bg-[var(--bg-primary)]/40 p-3 shadow-inner backdrop-blur-md sm:p-3.5">
        <div className="grid grid-cols-[1fr_auto_1fr] items-center gap-3">
          {/* 左侧：现场抓拍特写 */}
          <AvatarBox
            src={fieldCropPath}
            alt={t('card.siteCrop')}
            label={t('card.fieldCapture')}
            viewHdText={t('card.viewHd')}
            noImageText={t('card.noImage')}
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

          {/* 中央：科技感相似度仪表枢纽 */}
          <div className="flex flex-col items-center justify-center gap-1.5 px-0.5">
            <span className="font-mono text-[9px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
              {t('card.match')}
            </span>

            <div
              className={`flex h-12 w-12 flex-col items-center justify-center rounded-2xl border font-mono backdrop-blur-md transition-transform duration-300 group-hover/card:scale-105 ${simColorClass}`}
            >
              <span className="text-xs font-bold tracking-tight">{simPct}</span>
            </div>

            {/* 按需查看现场全景胶囊按钮 */}
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
                className="mt-0.5 flex items-center gap-1 rounded-full border border-[var(--border)]/80 bg-[var(--bg-surface)]/80 px-2 py-0.5 text-[9px] font-medium text-[var(--text-secondary)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] hover:shadow-xs active:scale-95"
                title={t('card.viewPanorama')}
              >
                <ImageIcon className="h-2.5 w-2.5" />
                <span>{t('card.viewPanorama')}</span>
              </button>
            ) : (
              <span className="text-[9px] text-[var(--text-muted)] opacity-60">
                {t('card.similarity')}
              </span>
            )}
          </div>

          {/* 右侧：底库档案登记照 */}
          <AvatarBox
            src={registeredPhotoRel}
            alt={t('card.registeredPhoto')}
            label={t('card.archivePhoto')}
            viewHdText={t('card.viewHd')}
            noImageText={t('card.noImage')}
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
        </div>
      </div>

      {/* 人员身份档案与时间元数据 */}
      <div className="relative z-10 space-y-2 border-t border-[var(--border)]/50 pt-2.5 text-xs">
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="truncate text-sm font-semibold tracking-tight text-[var(--text-primary)]">
              {recognition.subjectName || t('card.unknownSubject')}
            </span>
            <span className="rounded-md border border-[var(--border)]/40 bg-[var(--bg-secondary)]/60 px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
              ID: {recognition.subjectId || '-'}
            </span>
          </div>
          <div className="flex shrink-0 items-center gap-1 font-mono text-[11px] text-[var(--text-muted)] tabular-nums">
            <Clock className="h-3 w-3 opacity-60" />
            <span>{formatTimestamp(recognition.recognizedAt)}</span>
          </div>
        </div>

        {/* 现代 SaaS 动作底栏 (候选人抽屉 & 快速核验) */}
        <div className="flex items-center justify-between gap-2 pt-1">
          {candidates.length > 0 ? (
            <button
              type="button"
              onClick={() => onOpenReview(recognition)}
              className="flex items-center gap-1.5 rounded-xl border border-[var(--border)]/80 bg-[var(--bg-surface)]/80 px-2.5 py-1.5 font-mono text-[11px] font-medium text-[var(--text-secondary)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] hover:shadow-xs active:scale-95"
            >
              <Users className="h-3.5 w-3.5" />
              <span>
                Top-{candidates.length} {t('card.candidateList')}
              </span>
            </button>
          ) : (
            <div />
          )}

          {isPending && (
            <div className="flex items-center gap-2">
              <motion.button
                type="button"
                whileTap={{ scale: 0.96 }}
                onClick={() => onQuickReview(recognition, 'confirmed')}
                className="flex items-center gap-1 rounded-xl border border-[var(--status-success-border)] bg-[var(--status-success-soft)] px-3 py-1.5 text-xs font-semibold text-[var(--status-success)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--status-success)] hover:bg-[var(--status-success)] hover:text-white hover:shadow-[0_0_12px_var(--status-success-soft)]"
                title={t('card.passTop1')}
              >
                <Check className="h-3.5 w-3.5 stroke-[2.5]" />
                <span>{t('card.passTop1')}</span>
              </motion.button>
              <motion.button
                type="button"
                whileTap={{ scale: 0.96 }}
                onClick={() => onQuickReview(recognition, 'rejected')}
                className="flex items-center gap-1 rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] px-3 py-1.5 text-xs font-semibold text-[var(--status-danger)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--status-danger)] hover:bg-[var(--status-danger)] hover:text-white hover:shadow-[0_0_12px_var(--status-danger-soft)]"
                title={t('card.reject')}
              >
                <X className="h-3.5 w-3.5 stroke-[2.5]" />
                <span>{t('card.reject')}</span>
              </motion.button>
            </div>
          )}
        </div>
      </div>

      {/* 高清图片大图全屏预览灯箱 */}
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
    </div>
  )
})
