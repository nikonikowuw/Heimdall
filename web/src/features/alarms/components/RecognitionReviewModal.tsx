import React, { useState } from 'react'
import { Check, Users, X, ZoomIn } from 'lucide-react'
import { useDismissStack } from '../../../hooks/use-dismiss-stack'
import { evidenceApi } from '../../../lib/api'
import type { FaceCandidateItem, RecognitionRecord } from '../../../types'
import { formatCosineSimilarityPercent } from '@/lib/similarity'
import { formatTimestamp } from '../utils'
import { ImagePreviewModal } from './ImagePreviewModal'

export interface RecognitionReviewModalProps {
  recognition: RecognitionRecord
  cameraName?: string
  onClose: () => void
  onReview: (
    recognition: RecognitionRecord,
    status: 'confirmed' | 'rejected',
    candidate?: FaceCandidateItem,
  ) => void
  t: (key: string) => string
}

interface ReviewPreviewThumbProps {
  src?: string | null
  alt: string
  label?: string
  className: string
  title?: string
  noImageText: string
  onPreview?: () => void
}

function ReviewPreviewThumb({
  src,
  alt,
  label,
  className,
  title,
  noImageText,
  onPreview,
}: ReviewPreviewThumbProps): React.ReactElement {
  return (
    <div className="flex flex-col items-center gap-1">
      <div
        onClick={src ? onPreview : undefined}
        className={`group/img relative overflow-hidden rounded-xl border border-[var(--border)] bg-black shadow-xs ${className} ${
          src ? 'cursor-pointer hover:border-[var(--accent)] hover:shadow-md' : ''
        }`}
        title={src ? title : undefined}
      >
        {src ? (
          <>
            <img
              src={evidenceApi.getImageUrl(src)}
              alt={alt}
              className="h-full w-full object-cover transition-transform duration-200 group-hover/img:scale-105"
            />
            <div className="backdrop-blur-2xs absolute inset-0 flex items-center justify-center bg-black/40 opacity-0 transition-opacity duration-200 group-hover/img:opacity-100">
              <ZoomIn className="h-4 w-4 text-white" />
            </div>
          </>
        ) : (
          <div className="flex h-full items-center justify-center text-[10px] text-slate-500">
            {noImageText}
          </div>
        )}
      </div>
      {label && <span className="text-[10px] text-[var(--text-muted)]">{label}</span>}
    </div>
  )
}

const STATUS_LABEL_KEYS: Record<string, string> = {
  confirmed: 'card.statusConfirmed',
  rejected: 'card.statusRejected',
  pending_review: 'card.statusPendingReview',
}

export function RecognitionReviewModal({
  recognition,
  cameraName,
  onClose,
  onReview,
  t,
}: RecognitionReviewModalProps): React.ReactElement {
  const candidates = recognition.candidates || []
  const registeredPhotoRel = recognition.registeredPhotoPath || candidates[0]?.photoRelPath || ''
  const [compareCandidate, setCompareCandidate] = useState<FaceCandidateItem | null>(
    () => candidates[0] || null,
  )
  const [previewImage, setPreviewImage] = useState<{
    src: string
    title?: string
    subtitle?: string
  } | null>(null)

  // 浮层按栈响应 ESC，杜绝穿透
  useDismissStack(true, onClose)

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="relative flex max-h-[92vh] w-full max-w-4xl flex-col overflow-hidden rounded-3xl border border-[var(--border)] bg-[var(--bg-surface)] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-[var(--border)] px-6 py-4">
          <div className="flex items-center gap-2.5">
            <Users className="h-5 w-5 text-[var(--accent)]" />
            <h3 className="text-sm font-semibold text-[var(--text-primary)]">
              {t('card.reviewTitle')}
            </h3>
            <span className="font-mono text-xs text-[var(--text-muted)]">
              {recognition.recognitionId}
            </span>
          </div>
          <button
            onClick={onClose}
            aria-label={t('card.cancel')}
            className="rounded-xl p-1.5 text-[var(--text-muted)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-auto p-6">
          <div className="grid grid-cols-1 gap-6 md:grid-cols-3">
            {/* 左侧：现场特写与底库样本 */}
            <div className="flex flex-col gap-3 rounded-2xl border border-[var(--border)] bg-[var(--bg-base)] p-4">
              <span className="text-xs font-semibold text-[var(--text-secondary)]">
                {t('card.siteCrop')} & {t('card.registeredPhoto')}
              </span>
              <div className="grid grid-cols-2 gap-2">
                <ReviewPreviewThumb
                  src={recognition.fieldCropPath}
                  alt={t('card.siteCrop')}
                  label={t('card.siteCrop')}
                  className="aspect-square w-full"
                  title={t('card.viewHd')}
                  noImageText={t('card.noImage')}
                  onPreview={() => {
                    if (recognition.fieldCropPath) {
                      setPreviewImage({
                        src: evidenceApi.getImageUrl(recognition.fieldCropPath),
                        title: `${t('card.siteCrop')} · ${recognition.subjectName || recognition.cameraId}`,
                        subtitle: `${cameraName || recognition.cameraId} · ${formatTimestamp(recognition.recognizedAt)}`,
                      })
                    }
                  }}
                />
                <ReviewPreviewThumb
                  src={registeredPhotoRel}
                  alt={t('card.registeredPhoto')}
                  label={t('card.registeredPhoto')}
                  className="aspect-square w-full"
                  title={t('card.viewHd')}
                  noImageText={t('card.noImage')}
                  onPreview={() => {
                    if (registeredPhotoRel) {
                      setPreviewImage({
                        src: evidenceApi.getImageUrl(registeredPhotoRel),
                        title: `${t('card.registeredPhoto')}: ${recognition.subjectName || '底库样本'}`,
                        subtitle: `ID: ${recognition.subjectId || '-'}`,
                      })
                    }
                  }}
                />
              </div>
              <div className="space-y-1.5 pt-2 text-xs">
                <div className="flex justify-between">
                  <span className="text-[var(--text-muted)]">{t('modal.channel')}:</span>
                  <span
                    className="max-w-[200px] truncate font-medium text-[var(--text-primary)]"
                    title={cameraName || recognition.cameraId}
                  >
                    {cameraName || recognition.cameraId}
                  </span>
                </div>
                <div className="flex justify-between">
                  <span className="text-[var(--text-muted)]">{t('modal.time')}:</span>
                  <span className="font-mono text-[var(--text-primary)]">
                    {formatTimestamp(recognition.recognizedAt)}
                  </span>
                </div>
                <div className="flex justify-between">
                  <span className="text-[var(--text-muted)]">{t('card.currentStatus')}:</span>
                  <span className="font-semibold text-amber-500">
                    {t(STATUS_LABEL_KEYS[recognition.status] || 'card.statusPendingReview')}
                  </span>
                </div>
              </div>
            </div>

            {/* 右侧：对比视图与 Top-5 候选人列表 */}
            <div className="flex flex-col gap-3 md:col-span-2">
              {/* 侧并侧对比视图 (点击候选人时展开) */}
              {compareCandidate && (
                <div className="flex flex-col gap-2 rounded-2xl border border-[var(--accent)]/30 bg-[var(--accent)]/5 p-4">
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-semibold text-[var(--accent)]">
                      {t('card.compareView')}
                    </span>
                    <button
                      onClick={() => setCompareCandidate(null)}
                      className="rounded-lg p-1 text-[var(--text-muted)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  </div>
                  <div className="flex items-center justify-center gap-6">
                    <ReviewPreviewThumb
                      src={recognition.fieldCropPath}
                      alt={t('card.siteCrop')}
                      label={t('card.siteCrop')}
                      className="h-32 w-32"
                      title={t('card.viewHd')}
                      noImageText={t('card.noImage')}
                      onPreview={() => {
                        if (recognition.fieldCropPath) {
                          setPreviewImage({
                            src: evidenceApi.getImageUrl(recognition.fieldCropPath),
                            title: `${t('card.siteCrop')} · ${recognition.subjectName || recognition.cameraId}`,
                            subtitle: `${cameraName || recognition.cameraId} · ${formatTimestamp(recognition.recognizedAt)}`,
                          })
                        }
                      }}
                    />
                    <div className="flex flex-col items-center gap-1.5">
                      <ReviewPreviewThumb
                        src={compareCandidate.photoRelPath}
                        alt={compareCandidate.subjectName}
                        label={compareCandidate.subjectName}
                        className="h-32 w-32"
                        title={t('card.viewHd')}
                        noImageText={t('card.noImage')}
                        onPreview={() => {
                          if (compareCandidate.photoRelPath) {
                            setPreviewImage({
                              src: evidenceApi.getImageUrl(compareCandidate.photoRelPath),
                              title: `${t('card.candidateList')}: ${compareCandidate.subjectName}`,
                              subtitle: `ID: ${compareCandidate.subjectId} · ${t('card.similarity')}: ${formatCosineSimilarityPercent(compareCandidate.similarity)}`,
                            })
                          }
                        }}
                      />
                      <span className="font-mono text-[10px] font-bold text-emerald-500">
                        {formatCosineSimilarityPercent(compareCandidate.similarity)}
                      </span>
                    </div>
                  </div>
                </div>
              )}
              <div className="flex items-center justify-between">
                <span className="text-xs font-semibold text-[var(--text-secondary)]">
                  {t('card.candidateList')} ({candidates.length})
                </span>
                <span className="text-[10px] text-[var(--text-muted)]">
                  {t('card.sortedBySimilarity')}
                </span>
              </div>

              {candidates.length === 0 ? (
                <div className="flex h-48 flex-col items-center justify-center rounded-2xl border border-dashed border-[var(--border)] text-xs text-[var(--text-muted)]">
                  <Users className="mb-2 h-6 w-6 opacity-40" />
                  <span>{t('card.noCandidates')}</span>
                </div>
              ) : (
                <div className="space-y-2.5">
                  {candidates.map((cand, idx) => {
                    const scorePct = formatCosineSimilarityPercent(cand.similarity)
                    const isComparing = compareCandidate?.faceId === cand.faceId
                    return (
                      <div
                        key={cand.faceId || idx}
                        onClick={() => setCompareCandidate(isComparing ? null : cand)}
                        className={`flex cursor-pointer items-center justify-between gap-3 rounded-2xl border bg-[var(--bg-base)] p-3 transition-all ${
                          isComparing
                            ? 'border-[var(--accent)] ring-1 ring-[var(--accent)]/20'
                            : 'border-[var(--border)] hover:border-[var(--accent)]'
                        }`}
                      >
                        <div className="flex items-center gap-3">
                          <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-[var(--accent-soft)] font-mono text-xs font-bold text-[var(--accent)]">
                            #{cand.rank || idx + 1}
                          </span>
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
                            className={`group/cand relative h-12 w-12 shrink-0 overflow-hidden rounded-xl border border-[var(--border)] bg-black ${
                              cand.photoRelPath
                                ? 'cursor-pointer hover:border-[var(--accent)] hover:shadow-md'
                                : ''
                            }`}
                            title={cand.photoRelPath ? t('card.viewHd') : undefined}
                          >
                            {cand.photoRelPath ? (
                              <>
                                <img
                                  src={evidenceApi.getImageUrl(cand.photoRelPath)}
                                  alt={cand.subjectName}
                                  className="h-full w-full object-cover transition-transform duration-200 group-hover/cand:scale-105"
                                />
                                <div className="backdrop-blur-2xs absolute inset-0 flex items-center justify-center bg-black/40 opacity-0 transition-opacity duration-200 group-hover/cand:opacity-100">
                                  <ZoomIn className="h-3 w-3 text-white" />
                                </div>
                              </>
                            ) : (
                              <div className="flex h-full items-center justify-center text-[10px] text-slate-500">
                                {t('card.noImage')}
                              </div>
                            )}
                          </div>
                          <div>
                            <div className="text-xs font-semibold text-[var(--text-primary)]">
                              {cand.subjectName}
                            </div>
                            <div className="font-mono text-[10px] text-[var(--text-muted)]">
                              ID: {cand.subjectId}
                            </div>
                          </div>
                        </div>

                        <div className="flex items-center gap-3">
                          <div className="flex items-center gap-3">
                            <div className="flex flex-col items-end">
                              <span className="font-mono text-xs font-bold text-emerald-500">
                                {scorePct}%
                              </span>
                              <span className="text-[9px] text-[var(--text-muted)]">
                                {t('card.similarity')}
                              </span>
                            </div>
                            <button
                              onClick={(e) => {
                                e.stopPropagation()
                                onReview(recognition, 'confirmed', cand)
                              }}
                              className="flex items-center gap-1 rounded-xl border border-emerald-500/30 bg-emerald-500/10 px-2.5 py-1.5 text-xs font-medium text-emerald-500 transition-colors hover:bg-emerald-500 hover:text-white"
                              title={t('card.confirmCandidate')}
                            >
                              <Check className="h-3.5 w-3.5" />
                              <span>{t('card.confirmCandidate')}</span>
                            </button>
                          </div>
                        </div>
                      </div>
                    )
                  })}
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Footer Actions */}
        <div className="flex items-center justify-between border-t border-[var(--border)] bg-[var(--bg-secondary)]/50 px-6 py-4">
          <button
            onClick={() => onReview(recognition, 'rejected')}
            className="flex items-center gap-1.5 rounded-xl border border-rose-500/30 bg-rose-500/10 px-4 py-2 text-xs font-medium text-rose-500 transition-colors hover:bg-rose-500 hover:text-white"
          >
            <X className="h-4 w-4" />
            <span>{t('card.rejectMatch')}</span>
          </button>
          <div className="flex items-center gap-2">
            <button
              onClick={onClose}
              className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
            >
              {t('card.cancel')}
            </button>
            {candidates[0] && (
              <button
                onClick={() => onReview(recognition, 'confirmed', candidates[0])}
                className="flex items-center gap-1.5 rounded-xl bg-emerald-600 px-4 py-2 text-xs font-medium text-white shadow-sm transition-colors hover:bg-emerald-500"
              >
                <Check className="h-4 w-4" />
                <span>
                  {t('card.passTop1')} ({candidates[0].subjectName})
                </span>
              </button>
            )}
          </div>
        </div>
      </div>

      {/* 高清图片大图全屏预览灯箱 */}
      {previewImage && (
        <ImagePreviewModal
          src={previewImage.src}
          title={previewImage.title}
          subtitle={previewImage.subtitle}
          onClose={() => setPreviewImage(null)}
        />
      )}
    </div>
  )
}
