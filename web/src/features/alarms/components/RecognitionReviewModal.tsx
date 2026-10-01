import React, { useCallback, useState } from 'react'
import {
  Award,
  Camera,
  Check,
  CheckCircle2,
  Clock,
  Download,
  ExternalLink,
  Film,
  Image as ImageIcon,
  Loader2,
  RefreshCw,
  Sparkles,
  User,
  Users,
  X,
  ZoomIn,
} from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { ModalOverlay } from '@/components/ui/ModalOverlay'
import { evidenceApi } from '@/lib/api'
import type { FaceCandidateItem, RecognitionRecord } from '@/types'
import { formatCosineSimilarityPercent, getCosineSimilarityLevel } from '@/lib/similarity'
import { useEventRecording } from '../hooks/useEventRecording'
import { exportRecordingAsMp4 } from '../recordingExport'
import { formatTimestamp } from '../utils'
import { EventRecordingPlayer } from './EventRecordingPlayer'
import { ImagePreviewModal } from './ImagePreviewModal'
import { RECOGNITION_STATUS_STYLES, SIMILARITY_STYLES } from '../statusStyles'

export interface RecognitionReviewModalProps {
  recognition: RecognitionRecord
  cameraName?: string
  onClose: () => void
  onReview: (
    recognition: RecognitionRecord,
    status: 'confirmed' | 'rejected',
    candidate?: FaceCandidateItem,
  ) => void
  t: (key: string, options?: Record<string, unknown>) => string
}

interface MiniAvatarProps {
  src?: string | null
  alt: string
  label?: string
  className?: string
  onZoom?: () => void
  viewHdText: string
  noImageText: string
}

function MiniAvatar({
  src,
  alt,
  label,
  className = 'aspect-square w-full',
  onZoom,
  viewHdText,
  noImageText,
}: MiniAvatarProps): React.ReactElement {
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
        className={`group/avatar relative overflow-hidden rounded-2xl border border-[var(--border)] bg-black/90 shadow-xs transition-all duration-200 ${className} ${
          isClickable
            ? 'cursor-pointer hover:border-[var(--accent)] hover:shadow-md active:scale-98'
            : ''
        }`}
        title={isClickable ? viewHdText : undefined}
      >
        {src && !hasError ? (
          <>
            <img
              src={evidenceApi.getImageUrl(src)}
              alt={alt}
              loading="lazy"
              decoding="async"
              onError={() => setHasError(true)}
              className="h-full w-full object-cover transition-transform duration-300 group-hover/avatar:scale-105"
            />
            <div className="absolute inset-0 flex items-center justify-center bg-black/40 opacity-0 transition-opacity duration-200 group-hover/avatar:opacity-100">
              <span className="flex items-center gap-1 rounded-xl border border-white/20 bg-black/70 px-2 py-1 text-[10px] font-medium text-white shadow-lg backdrop-blur-md">
                <ZoomIn className="h-3 w-3" />
                <span>{viewHdText}</span>
              </span>
            </div>
          </>
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-1 p-2 text-[var(--text-muted)]">
            <User className="h-6 w-6 opacity-30" />
            <span className="text-[10px] opacity-60">{noImageText}</span>
          </div>
        )}
      </div>
    </div>
  )
}

const STATUS_CONFIG = {
  confirmed: {
    ...RECOGNITION_STATUS_STYLES.confirmed,
    labelKey: 'card.statusConfirmed',
  },
  pending_review: {
    ...RECOGNITION_STATUS_STYLES.pending_review,
    labelKey: 'card.statusPendingReview',
  },
  rejected: {
    ...RECOGNITION_STATUS_STYLES.rejected,
    labelKey: 'card.statusRejected',
  },
} as const

export function RecognitionReviewModal({
  recognition,
  cameraName,
  onClose,
  onReview,
  t,
}: RecognitionReviewModalProps): React.ReactElement {
  const { t: tr } = useTranslation('recording')
  const candidates = recognition.candidates || []
  const [selectedCandidate, setSelectedCandidate] = useState<FaceCandidateItem | null>(
    () => candidates[0] || null,
  )
  const [previewImage, setPreviewImage] = useState<{
    src: string
    title?: string
    subtitle?: string
    bboxJson?: string | null
  } | null>(null)

  // 识别命中是录像触发源之一：有识别结果的记录才可能关联到录像片段
  const {
    recording,
    error: recordingError,
    retry: retryRecording,
  } = useEventRecording('recognition', recognition.recognitionId)
  const [mediaMode, setMediaMode] = useState<'image' | 'video'>('image')
  const [exportState, setExportState] = useState<'idle' | 'exporting' | 'failed'>('idle')
  const activeEventOffsetMs =
    recording?.events.find((item) => item.eventId === recognition.recognitionId)?.offsetMs ?? 0

  // 换记录时回到证据图模式，避免沿用上一条的媒体选择
  const [mediaRecognitionId, setMediaRecognitionId] = useState(recognition.recognitionId)
  if (mediaRecognitionId !== recognition.recognitionId) {
    setMediaRecognitionId(recognition.recognitionId)
    setMediaMode('image')
    setExportState('idle')
  }

  const handleExport = useCallback(() => {
    if (!recording || exportState === 'exporting') return
    setExportState('exporting')
    exportRecordingAsMp4(recording)
      .then(() => setExportState('idle'))
      .catch(() => setExportState('failed'))
  }, [recording, exportState])

  // 浮层按栈响应 ESC，杜绝穿透

  const activeCandidate = selectedCandidate || candidates[0]
  const activeSimilarity = activeCandidate?.similarity ?? recognition.similarity
  const simPct = formatCosineSimilarityPercent(activeSimilarity, 0)
  const activeLevel = getCosineSimilarityLevel(activeSimilarity)
  const fieldImagePath = recognition.fieldImagePath
  const fieldCropPath = recognition.fieldCropPath || fieldImagePath || ''

  const statusConfig =
    STATUS_CONFIG[recognition.status as keyof typeof STATUS_CONFIG] ?? STATUS_CONFIG.pending_review

  const activeSimClass = SIMILARITY_STYLES[activeLevel]

  return (
    <ModalOverlay
      isOpen
      onClose={onClose}
      ariaLabel={t('card.reviewTitle')}
      panelClassName="modal-surface--extra-wide max-h-[92vh] select-none"
    >
      {/* 顶部高光反射线 */}
      <div className="pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-white/25 to-transparent" />

      {/* 顶部综合 HUD 信息栏 */}
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border)]/60 px-6 py-4">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl border border-[var(--accent)]/30 bg-[var(--accent-soft)] text-[var(--accent)] shadow-xs">
            <Users className="h-5 w-5" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h3 className="text-sm font-semibold tracking-tight text-[var(--text-primary)]">
                {t('card.reviewTitle')}
              </h3>
              <span className="rounded-md border border-[var(--border)]/60 bg-[var(--bg-secondary)]/50 px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
                ID: {recognition.recognitionId}
              </span>
            </div>
            <p className="text-[11px] text-[var(--text-muted)]">{t('card.sortedBySimilarity')}</p>
          </div>
        </div>

        <div className="flex items-center gap-2.5">
          {/* 状态指示 */}
          <span
            className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 font-mono text-[10px] font-semibold tracking-wide shadow-2xs backdrop-blur-xs ${statusConfig.chipClass}`}
          >
            <span className={`h-1.5 w-1.5 rounded-full ${statusConfig.dotClass}`} />
            {t(statusConfig.labelKey)}
          </span>

          {/* 通道点位胶囊 */}
          <div
            className="hidden items-center gap-1.5 rounded-full border border-[var(--border)]/60 bg-[var(--bg-secondary)]/40 px-2.5 py-1 text-xs text-[var(--text-secondary)] backdrop-blur-xs sm:flex"
            title={cameraName || recognition.cameraId}
          >
            <Camera className="h-3 w-3 text-[var(--text-muted)]" />
            <span className="max-w-[130px] truncate font-medium">
              {cameraName || recognition.cameraId}
            </span>
          </div>

          {/* 抓拍时间 */}
          <div className="hidden items-center gap-1 font-mono text-[11px] text-[var(--text-muted)] tabular-nums md:flex">
            <Clock className="h-3 w-3 opacity-60" />
            <span>{formatTimestamp(recognition.recognizedAt)}</span>
          </div>

          {/* 录像查询失败：给出可操作的重试，不静默当作无录像 */}
          {recordingError && !recording && (
            <button
              type="button"
              onClick={retryRecording}
              title={recordingError}
              className="border-status-danger/40 bg-status-danger/15 text-status-danger hover:bg-status-danger/25 flex items-center gap-1.5 rounded-xl border px-2.5 py-1 text-xs transition-colors"
            >
              <RefreshCw className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">
                {tr('playback.loadRetry', { defaultValue: '录像加载失败，重试' })}
              </span>
            </button>
          )}

          {/* 媒体切换：仅当该识别命中已保存录像时出现 */}
          {recording && (
            <div className="flex items-center gap-0.5 rounded-xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/40 p-0.5 text-xs">
              <button
                type="button"
                onClick={() => setMediaMode('image')}
                aria-pressed={mediaMode === 'image'}
                className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 transition-colors ${
                  mediaMode === 'image'
                    ? 'bg-[var(--bg-surface-solid)] text-[var(--text-primary)] shadow-xs'
                    : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                }`}
              >
                <ImageIcon className="h-3.5 w-3.5" />
                <span className="hidden sm:inline">
                  {tr('playback.mediaImage', { defaultValue: '证据图' })}
                </span>
              </button>
              <button
                type="button"
                onClick={() => setMediaMode('video')}
                aria-pressed={mediaMode === 'video'}
                className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 transition-colors ${
                  mediaMode === 'video'
                    ? 'bg-[var(--bg-surface-solid)] text-[var(--text-primary)] shadow-xs'
                    : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                }`}
              >
                <Film className="h-3.5 w-3.5" />
                <span className="hidden sm:inline">
                  {tr('playback.mediaVideo', { defaultValue: '录像回放' })}
                </span>
              </button>
            </div>
          )}

          {mediaMode === 'video' && recording && (
            <button
              type="button"
              disabled={exportState === 'exporting'}
              onClick={handleExport}
              title={exportState === 'failed' ? tr('playback.exportFailed') : tr('playback.export')}
              className={`flex items-center gap-1.5 rounded-xl border px-2.5 py-1 text-xs transition-colors disabled:opacity-60 ${
                exportState === 'failed'
                  ? 'border-status-danger/40 bg-status-danger/15 text-status-danger'
                  : 'border-[var(--border)]/70 bg-[var(--bg-secondary)]/40 text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              {exportState === 'exporting' ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Download className="h-3.5 w-3.5" />
              )}
              <span className="hidden sm:inline">
                {exportState === 'exporting'
                  ? tr('playback.exporting')
                  : exportState === 'failed'
                    ? tr('playback.exportRetry')
                    : tr('playback.export')}
              </span>
            </button>
          )}

          {/* 关闭按钮 */}
          <button
            onClick={onClose}
            aria-label={t('card.cancel')}
            className="rounded-xl border border-[var(--border)]/80 bg-[var(--bg-surface)]/80 p-1.5 text-[var(--text-muted)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      </div>

      {/* 内容区：左侧实时对比基准舱 VS 右侧 Top-5 候选池；切到录像则整区让位给播放器 */}
      <div className="flex-1 overflow-y-auto p-5 sm:p-6">
        {mediaMode === 'video' && recording ? (
          <div className="flex min-h-[44vh] items-center justify-center">
            <EventRecordingPlayer
              recording={recording}
              initialOffsetMs={activeEventOffsetMs}
              className="max-h-[62vh] w-full max-w-4xl rounded-xl bg-black shadow-2xl"
            />
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-6 lg:grid-cols-12">
            {/* 左侧：实时双头像镜面对比舱 (占 5 列) */}
            <div className="flex flex-col justify-between gap-4 rounded-2xl border border-[var(--border)]/60 bg-[var(--bg-primary)]/40 p-4.5 shadow-inner backdrop-blur-md lg:col-span-5">
              <div>
                <div className="flex items-center justify-between pb-3">
                  <span className="flex items-center gap-1.5 text-xs font-semibold tracking-tight text-[var(--text-primary)]">
                    <Sparkles className="h-3.5 w-3.5 text-[var(--accent)]" />
                    <span>{t('card.compareBenchmark')}</span>
                  </span>
                  <span className="font-mono text-[11px] text-[var(--text-muted)]">
                    1 : 1 Live Verify
                  </span>
                </div>

                {/* 双头像对比舞台 */}
                <div className="grid grid-cols-[1fr_auto_1fr] items-center gap-3 rounded-2xl border border-[var(--border)]/40 bg-[var(--bg-surface)]/50 p-3.5 shadow-xs backdrop-blur-xs">
                  {/* 左像：现场抓拍特写 */}
                  <MiniAvatar
                    src={fieldCropPath}
                    alt={t('card.siteCrop')}
                    label={t('card.fieldCapture')}
                    viewHdText={t('card.viewHd')}
                    noImageText={t('card.noImage')}
                    onZoom={() => {
                      if (fieldCropPath) {
                        setPreviewImage({
                          src: evidenceApi.getImageUrl(fieldCropPath),
                          title: `${t('card.siteCrop')} · ${recognition.subjectName || recognition.cameraId}`,
                          subtitle: `${cameraName || recognition.cameraId} · ${formatTimestamp(recognition.recognizedAt)}`,
                        })
                      }
                    }}
                  />

                  {/* 中间：实时相似度指示仪表 */}
                  <div className="flex flex-col items-center justify-center gap-1.5 px-0.5">
                    <span className="font-mono text-[9px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
                      {t('card.match')}
                    </span>
                    <div
                      className={`flex h-12 w-12 flex-col items-center justify-center rounded-2xl border font-mono backdrop-blur-md transition-transform duration-300 ${activeSimClass}`}
                    >
                      <span className="text-xs font-bold tracking-tight">{simPct}</span>
                    </div>
                    <span className="text-[9px] font-medium text-[var(--text-muted)]">
                      {activeCandidate?.rank ? `#${activeCandidate.rank}` : '#1'}
                    </span>
                  </div>

                  {/* 右像：当前比对候选人照片 */}
                  <MiniAvatar
                    src={activeCandidate?.photoRelPath || recognition.registeredPhotoPath}
                    alt={activeCandidate?.subjectName || t('card.registeredPhoto')}
                    label={t('card.archivePhoto')}
                    viewHdText={t('card.viewHd')}
                    noImageText={t('card.noImage')}
                    onZoom={() => {
                      const p = activeCandidate?.photoRelPath || recognition.registeredPhotoPath
                      if (p) {
                        setPreviewImage({
                          src: evidenceApi.getImageUrl(p),
                          title: `${t('card.registeredPhoto')}: ${activeCandidate?.subjectName || recognition.subjectName}`,
                          subtitle: `ID: ${activeCandidate?.subjectId || recognition.subjectId || '-'}`,
                        })
                      }
                    }}
                  />
                </div>

                {/* 候选目标身份摘要卡 */}
                <div className="mt-3.5 space-y-2 rounded-xl border border-[var(--border)]/40 bg-[var(--bg-surface)]/40 p-3 text-xs">
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">{t('columns.subject')}:</span>
                    <span className="font-semibold text-[var(--text-primary)]">
                      {activeCandidate?.subjectName ||
                        recognition.subjectName ||
                        t('card.unknownSubject')}
                    </span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">ID:</span>
                    <span className="font-mono text-[var(--text-secondary)]">
                      {activeCandidate?.subjectId || recognition.subjectId || '-'}
                    </span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">{t('card.similarity')}:</span>
                    <span className="font-mono font-bold text-[var(--status-success)]">
                      {simPct}
                    </span>
                  </div>
                </div>
              </div>

              {/* 现场全景背景按需查看 (不占大面积，一键预览) */}
              {fieldImagePath && (
                <button
                  type="button"
                  onClick={() => {
                    setPreviewImage({
                      src: evidenceApi.getImageUrl(fieldImagePath),
                      bboxJson: recognition.fieldBboxJson,
                      title: `${t('card.sitePanorama')} · ${recognition.subjectName || recognition.cameraId}`,
                      subtitle: `${cameraName || recognition.cameraId} · ${formatTimestamp(recognition.recognizedAt)}`,
                    })
                  }}
                  className="flex w-full items-center justify-between rounded-xl border border-[var(--border)]/70 bg-[var(--bg-surface)]/60 p-2.5 text-xs text-[var(--text-secondary)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)]/30 hover:text-[var(--accent)] active:scale-98"
                >
                  <span className="flex items-center gap-2">
                    <ImageIcon className="h-4 w-4 text-[var(--accent)]" />
                    <span className="font-medium text-[var(--text-primary)]">
                      {t('card.sitePanorama')}
                    </span>
                  </span>
                  <span className="flex items-center gap-1 text-[11px] font-medium text-[var(--accent)]">
                    <span>{t('card.viewHd')}</span>
                    <ExternalLink className="h-3 w-3" />
                  </span>
                </button>
              )}
            </div>

            {/* 右侧：Top-5 相似度候选人池 (占 7 列) */}
            <div className="flex flex-col gap-3 lg:col-span-7">
              <div className="flex items-center justify-between">
                <span className="text-xs font-semibold tracking-tight text-[var(--text-secondary)]">
                  {t('card.candidatePool')} ({candidates.length})
                </span>
                <span className="text-[11px] text-[var(--text-muted)]">
                  {t('card.sortedBySimilarity')}
                </span>
              </div>

              {candidates.length === 0 ? (
                <div className="flex h-64 flex-col items-center justify-center rounded-2xl border border-dashed border-[var(--border)] p-6 text-center text-xs text-[var(--text-muted)]">
                  <Users className="mb-2 h-8 w-8 opacity-30" />
                  <span>{t('card.noCandidates')}</span>
                </div>
              ) : (
                <div className="space-y-2.5">
                  {candidates.map((cand, idx) => {
                    const scorePct = formatCosineSimilarityPercent(cand.similarity, 0)
                    const isSelected = activeCandidate?.faceId === cand.faceId
                    const isTop1 = idx === 0
                    const candLevel = getCosineSimilarityLevel(cand.similarity)
                    const simRatio = Math.max(0, Math.min(100, (cand.similarity ?? 0) * 100))

                    return (
                      <div
                        key={cand.faceId || idx}
                        onClick={() => setSelectedCandidate(cand)}
                        className={`relative flex cursor-pointer items-center justify-between gap-3.5 rounded-2xl border p-3 shadow-xs backdrop-blur-md transition-all duration-200 ${
                          isSelected
                            ? 'border-[var(--accent)] bg-[var(--accent-soft)]/20 shadow-md ring-1 ring-[var(--accent)]/30'
                            : 'border-[var(--border)]/70 bg-[var(--bg-surface)]/50 hover:border-[var(--accent)]/50 hover:bg-[var(--bg-surface)]'
                        }`}
                      >
                        <div className="flex min-w-0 items-center gap-3">
                          {/* 排名勋章 */}
                          <div className="flex shrink-0 flex-col items-center justify-center">
                            {isTop1 ? (
                              <span className="flex h-7 w-7 items-center justify-center rounded-xl border border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] font-mono text-xs font-bold text-[var(--status-warning)] shadow-xs">
                                <Award className="h-4 w-4" />
                              </span>
                            ) : (
                              <span className="flex h-7 w-7 items-center justify-center rounded-xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/50 font-mono text-xs font-semibold text-[var(--text-muted)]">
                                #{cand.rank || idx + 1}
                              </span>
                            )}
                          </div>

                          {/* 候选人头像 */}
                          <div
                            onClick={(e) => {
                              e.stopPropagation()
                              if (cand.photoRelPath) {
                                setPreviewImage({
                                  src: evidenceApi.getImageUrl(cand.photoRelPath),
                                  title: `${t('card.candidateList')}: ${cand.subjectName}`,
                                  subtitle: `#${cand.rank || idx + 1} · ID: ${cand.subjectId}`,
                                })
                              }
                            }}
                            className={`group/thumb relative h-12 w-12 shrink-0 overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--video-surface)] shadow-xs ${
                              cand.photoRelPath ? 'cursor-pointer hover:border-[var(--accent)]' : ''
                            }`}
                            title={cand.photoRelPath ? t('card.viewHd') : undefined}
                          >
                            {cand.photoRelPath ? (
                              <>
                                <img
                                  src={evidenceApi.getImageUrl(cand.photoRelPath)}
                                  alt={cand.subjectName}
                                  loading="lazy"
                                  decoding="async"
                                  className="h-full w-full object-cover transition-transform duration-200 group-hover/thumb:scale-105"
                                />
                                <div className="absolute inset-0 flex items-center justify-center bg-[var(--overlay-scrim)] opacity-0 transition-opacity duration-200 group-hover/thumb:opacity-100">
                                  <ZoomIn className="h-3 w-3 text-white" />
                                </div>
                              </>
                            ) : (
                              <div className="flex h-full items-center justify-center text-[10px] text-[var(--text-muted)]">
                                {t('card.noImage')}
                              </div>
                            )}
                          </div>

                          {/* 身份与置信度进度条 */}
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center gap-2">
                              <span className="truncate text-xs font-semibold text-[var(--text-primary)]">
                                {cand.subjectName}
                              </span>
                              {isTop1 && (
                                <span className="rounded-md border border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] px-1.5 py-0.5 font-mono text-[9px] font-semibold text-[var(--status-warning)]">
                                  {t('card.topMatchBadge')}
                                </span>
                              )}
                            </div>
                            <div className="mt-0.5 font-mono text-[10px] text-[var(--text-muted)]">
                              ID: {cand.subjectId}
                            </div>
                            {/* 置信度条 */}
                            <div className="mt-1.5 flex items-center gap-2">
                              <div className="h-1.5 w-24 overflow-hidden rounded-full bg-[var(--bg-secondary)] sm:w-28">
                                <div
                                  className={`h-full rounded-full transition-all duration-300 ${
                                    candLevel === 'high'
                                      ? 'bg-[var(--status-success)]'
                                      : candLevel === 'medium'
                                        ? 'bg-[var(--status-warning)]'
                                        : 'bg-[var(--status-danger)]'
                                  }`}
                                  style={{ width: `${simRatio}%` }}
                                />
                              </div>
                              <span
                                className={`font-mono text-[10px] font-bold ${
                                  candLevel === 'high'
                                    ? 'text-[var(--status-success)]'
                                    : candLevel === 'medium'
                                      ? 'text-[var(--status-warning)]'
                                      : 'text-[var(--status-danger)]'
                                }`}
                              >
                                {scorePct}
                              </span>
                            </div>
                          </div>
                        </div>

                        {/* 确认为此人操作按钮 */}
                        <div className="shrink-0 pl-1">
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation()
                              onReview(recognition, 'confirmed', cand)
                            }}
                            className="flex items-center gap-1 rounded-xl border border-[var(--status-success-border)] bg-[var(--status-success-soft)] px-2.5 py-1.5 text-xs font-semibold text-[var(--status-success)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--status-success)] hover:bg-[var(--status-success-solid)] hover:text-white hover:shadow-[0_0_12px_var(--status-success-soft)] active:scale-95"
                            title={t('card.confirmCandidate')}
                          >
                            <Check className="h-3.5 w-3.5 stroke-[2.5]" />
                            <span className="hidden sm:inline">{t('card.adoptCandidate')}</span>
                          </button>
                        </div>
                      </div>
                    )
                  })}
                </div>
              )}
            </div>
          </div>
        )}
      </div>

      {/* 底部决策控制坞 (Decision Dock) */}
      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border)]/60 bg-[var(--bg-secondary)]/40 px-6 py-4 backdrop-blur-xl">
        <motion.button
          type="button"
          whileTap={{ scale: 0.96 }}
          onClick={() => onReview(recognition, 'rejected')}
          className="flex items-center gap-1.5 rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] px-4 py-2 text-xs font-semibold text-[var(--status-danger)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--status-danger)] hover:bg-[var(--status-danger-solid)] hover:text-white hover:shadow-[0_0_12px_var(--status-danger-soft)]"
        >
          <X className="h-4 w-4 stroke-[2.5]" />
          <span>{t('card.rejectMatch')}</span>
        </motion.button>

        <div className="flex items-center gap-2.5">
          <button
            type="button"
            onClick={onClose}
            className="rounded-xl border border-[var(--border)]/80 bg-[var(--bg-surface)]/80 px-4 py-2 text-xs font-medium text-[var(--text-secondary)] shadow-2xs backdrop-blur-xs transition-all hover:border-[var(--accent)] hover:text-[var(--accent)] active:scale-95"
          >
            {t('card.cancel')}
          </button>

          {activeCandidate && (
            <motion.button
              type="button"
              whileTap={{ scale: 0.96 }}
              onClick={() => onReview(recognition, 'confirmed', activeCandidate)}
              className="flex items-center gap-1.5 rounded-xl border border-[var(--status-success-border)] bg-[var(--status-success-solid)] px-4.5 py-2 text-xs font-semibold text-white shadow-md transition-all hover:opacity-90"
            >
              <CheckCircle2 className="h-4 w-4 stroke-[2.5]" />
              <span>
                {t('card.confirmCandidate')} ({activeCandidate.subjectName})
              </span>
            </motion.button>
          )}
        </div>
      </div>
      {/* 高清图片大图全屏预览灯箱 */}
      <AnimatePresence>
        {previewImage && (
          <ImagePreviewModal
            src={previewImage.src}
            title={previewImage.title}
            subtitle={previewImage.subtitle}
            bboxJson={previewImage.bboxJson}
            onClose={() => setPreviewImage(null)}
          />
        )}
      </AnimatePresence>
    </ModalOverlay>
  )
}
