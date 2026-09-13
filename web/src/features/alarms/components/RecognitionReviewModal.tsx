import React, { useState } from 'react'
import { Check, Users, X } from 'lucide-react'
import { useDismissStack } from '../../../hooks/use-dismiss-stack'
import { evidenceApi } from '../../../lib/api'
import type { FaceCandidateItem, RecognitionRecord } from '../../../types'
import { formatTimestamp } from '../utils'

export interface RecognitionReviewModalProps {
  recognition: RecognitionRecord
  onClose: () => void
  onReview: (
    recognition: RecognitionRecord,
    status: 'confirmed' | 'rejected',
    candidate?: FaceCandidateItem,
  ) => void
  t: (key: string) => string
}

export function RecognitionReviewModal({
  recognition,
  onClose,
  onReview,
  t,
}: RecognitionReviewModalProps): React.ReactElement {
  const candidates = recognition.candidates || []
  const registeredPhotoRel = recognition.registeredPhotoPath || candidates[0]?.photoRelPath || ''
  const [compareCandidate, setCompareCandidate] = useState<FaceCandidateItem | null>(
    () => candidates[0] || null,
  )

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
                <div className="flex flex-col items-center gap-1">
                  <div className="aspect-square w-full overflow-hidden rounded-xl border border-[var(--border)] bg-black shadow-xs">
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
                <div className="flex flex-col items-center gap-1">
                  <div className="aspect-square w-full overflow-hidden rounded-xl border border-[var(--border)] bg-black shadow-xs">
                    {registeredPhotoRel ? (
                      <img
                        src={evidenceApi.getImageUrl(registeredPhotoRel)}
                        alt={t('card.registeredPhoto')}
                        className="h-full w-full object-cover"
                      />
                    ) : (
                      <div className="flex h-full items-center justify-center text-[10px] text-slate-500">
                        {t('card.noImage')}
                      </div>
                    )}
                  </div>
                  <span className="text-[10px] text-[var(--text-muted)]">
                    {t('card.registeredPhoto')}
                  </span>
                </div>
              </div>
              <div className="space-y-1.5 pt-2 text-xs">
                <div className="flex justify-between">
                  <span className="text-[var(--text-muted)]">{t('modal.channel')}:</span>
                  <span className="font-mono text-[var(--text-primary)]">
                    {recognition.cameraId}
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
                    {recognition.status === 'confirmed'
                      ? t('card.statusConfirmed')
                      : recognition.status === 'rejected'
                        ? t('card.statusRejected')
                        : t('card.statusPendingReview')}
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
                    <div className="flex flex-col items-center gap-1.5">
                      <span className="text-[10px] font-medium text-[var(--text-muted)]">
                        {t('card.siteCrop')}
                      </span>
                      <div className="h-32 w-32 overflow-hidden rounded-xl border border-[var(--border)] bg-black shadow-sm">
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
                    </div>
                    <div className="flex flex-col items-center gap-1.5">
                      <span className="text-[10px] font-medium text-[var(--text-muted)]">
                        {compareCandidate.subjectName}
                      </span>
                      <div className="h-32 w-32 overflow-hidden rounded-xl border border-[var(--border)] bg-black shadow-sm">
                        {compareCandidate.photoRelPath ? (
                          <img
                            src={evidenceApi.getImageUrl(compareCandidate.photoRelPath)}
                            alt={compareCandidate.subjectName}
                            className="h-full w-full object-cover"
                          />
                        ) : (
                          <div className="flex h-full items-center justify-center text-[10px] text-slate-500">
                            {t('card.noImage')}
                          </div>
                        )}
                      </div>
                      <span className="font-mono text-[10px] font-bold text-emerald-500">
                        {(compareCandidate.similarity * 100).toFixed(1)}%
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
                    const scorePct = (cand.similarity * 100).toFixed(1)
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
                          <div className="h-12 w-12 shrink-0 overflow-hidden rounded-xl border border-[var(--border)] bg-black">
                            {cand.photoRelPath ? (
                              <img
                                src={evidenceApi.getImageUrl(cand.photoRelPath)}
                                alt={cand.subjectName}
                                className="h-full w-full object-cover"
                              />
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

        {/* Footer */}
        <div className="flex items-center justify-between border-t border-[var(--border)] px-6 py-4">
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
              className="rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)]"
            >
              {t('card.cancel')}
            </button>
            {candidates.length > 0 && (
              <button
                onClick={() => onReview(recognition, 'confirmed', candidates[0])}
                className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-medium text-white shadow-xs transition-opacity hover:opacity-90"
              >
                <Check className="h-4 w-4" />
                <span>{t('card.passTop1')}</span>
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
