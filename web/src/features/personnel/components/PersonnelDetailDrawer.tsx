import React, { useState, useRef, useEffect, useCallback } from 'react'
import {
  X,
  Star,
  Trash2,
  UploadCloud,
  Loader2,
  AlertCircle,
  Clock,
  IdCard as IdCardIcon,
  Tag,
  CheckCircle2,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { evidenceApi, personnelApi } from '../../../lib/api'
import type { PersonnelDetail, GalleryFace } from '../../../types'
import { formatTimestamp } from '../../../lib/time'

interface PersonnelDetailDrawerProps {
  isOpen: boolean
  subjectId: string | null
  autoOpenUpload?: boolean
  onClose: () => void
  onUpdate: () => void
}

interface QualityGrade {
  label: string
  colorClass: string
  bgClass: string
  borderClass: string
  barClass: string
  scorePercent: number
}

function getQualityGrade(score: number, t: (key: string) => string): QualityGrade {
  const scorePercent = Math.round(score * 100)
  if (scorePercent >= 80) {
    return {
      label: t('quality.excellent'),
      colorClass: 'text-emerald-400',
      bgClass: 'bg-emerald-500/10',
      borderClass: 'border-emerald-500/30',
      barClass: 'bg-emerald-500',
      scorePercent,
    }
  }
  if (scorePercent >= 65) {
    return {
      label: t('quality.good'),
      colorClass: 'text-sky-400',
      bgClass: 'bg-sky-500/10',
      borderClass: 'border-sky-500/30',
      barClass: 'bg-sky-400',
      scorePercent,
    }
  }
  return {
    label: t('quality.fair'),
    colorClass: 'text-amber-400',
    bgClass: 'bg-amber-500/10',
    borderClass: 'border-amber-500/30',
    barClass: 'bg-amber-400',
    scorePercent,
  }
}

export const PersonnelDetailDrawer: React.FC<PersonnelDetailDrawerProps> = ({
  isOpen,
  subjectId,
  autoOpenUpload,
  onClose,
  onUpdate,
}) => {
  const { t } = useTranslation(['personnel', 'common'])
  const [detail, setDetail] = useState<PersonnelDetail | null>(null)
  const [loading, setLoading] = useState(false)
  const [actionLoading, setActionLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [activeTab, setActiveTab] = useState<'original' | 'aligned'>('original')
  const [faceToDelete, setFaceToDelete] = useState<GalleryFace | null>(null)

  const fileInputRef = useRef<HTMLInputElement>(null)

  const fetchDetail = useCallback(
    async (id: string) => {
      setLoading(true)
      setError(null)
      try {
        const data = await personnelApi.getDetail(id)
        setDetail(data)
      } catch (err: unknown) {
        setError(err instanceof Error ? err.message : t('errors.failedToLoad'))
      } finally {
        setLoading(false)
      }
    },
    [t],
  )

  useEffect(() => {
    if (isOpen && subjectId) {
      fetchDetail(subjectId).then(() => {
        if (autoOpenUpload) {
          setTimeout(() => {
            fileInputRef.current?.click()
          }, 150)
        }
      })
    } else {
      setDetail(null)
    }
  }, [isOpen, subjectId, autoOpenUpload, fetchDetail])

  if (!isOpen) return null

  const handleSetPrimary = async (face: GalleryFace) => {
    if (!detail || face.isPrimary) return
    setActionLoading(true)
    setError(null)
    try {
      const updated = await personnelApi.setPrimaryFace(detail.subjectId, face.faceId)
      setDetail(updated)
      onUpdate()
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : t('errors.failedToSave'))
    } finally {
      setActionLoading(false)
    }
  }

  const handleDeleteFace = (face: GalleryFace) => {
    if (!detail) return
    if (detail.faces.length <= 1) {
      setError(t('errors.cannotDeleteLastFace'))
      return
    }
    setFaceToDelete(face)
  }

  const handleConfirmDeleteFace = async () => {
    if (!detail || !faceToDelete) return
    setActionLoading(true)
    setError(null)
    try {
      const updated = await personnelApi.deleteFace(detail.subjectId, faceToDelete.faceId)
      setDetail(updated)
      setFaceToDelete(null)
      onUpdate()
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : t('errors.failedToDelete'))
    } finally {
      setActionLoading(false)
    }
  }

  const handleAddPhotos = async (e: React.ChangeEvent<HTMLInputElement>) => {
    if (!detail) return
    const files = Array.from(e.target.files || [])
    if (!files.length) return

    const availableSlots = 5 - detail.faces.length
    if (files.length > availableSlots) {
      setError(t('errors.maxPhotosExceeded'))
      return
    }

    setActionLoading(true)
    setError(null)
    try {
      const formData = new FormData()
      for (const f of files) {
        formData.append('images', f)
      }
      const updated = await personnelApi.addFaces(detail.subjectId, formData)
      setDetail(updated)
      onUpdate()
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : t('errors.failedToSave'))
    } finally {
      setActionLoading(false)
      if (fileInputRef.current) {
        fileInputRef.current.value = ''
      }
    }
  }

  return (
    <div className="animate-in fade-in fixed inset-0 z-50 flex justify-end bg-black/60 backdrop-blur-xs duration-200">
      <div className="animate-in slide-in-from-right relative flex h-full w-full max-w-lg flex-col border-l border-[var(--border)] bg-[var(--bg-secondary)] shadow-2xl duration-300">
        {/* 顶部栏 */}
        <div className="flex items-center justify-between border-b border-[var(--border)] px-6 py-4">
          <div className="flex items-center gap-2">
            <h2 className="text-base font-semibold text-[var(--text-primary)]">
              {t('actions.viewDetails')}
            </h2>
            {detail && (
              <span className="rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 font-mono text-[10px] font-medium text-emerald-400">
                {t('drawer.samplesCount', { current: detail.faces.length })}
              </span>
            )}
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg p-1.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-tertiary)] hover:text-[var(--text-primary)]"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* 抽屉主体 */}
        <div className="flex-1 space-y-6 overflow-y-auto p-6">
          {loading ? (
            <div className="flex h-64 items-center justify-center text-[var(--text-muted)]">
              <Loader2 className="h-6 w-6 animate-spin text-emerald-500" />
            </div>
          ) : detail ? (
            <>
              {error && (
                <div className="flex items-start gap-2.5 rounded-xl border border-red-500/30 bg-red-500/10 p-3 text-xs text-red-400">
                  <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
                  <span>{error}</span>
                </div>
              )}

              {/* 档案概览卡片 */}
              <div className="rounded-2xl border border-[var(--border)] bg-[var(--bg-tertiary)] p-4 shadow-xs">
                <div className="flex items-center gap-4">
                  <div className="relative h-16 w-16 shrink-0 overflow-hidden rounded-xl border border-[var(--border)] bg-black/60">
                    {detail.primaryPhotoPath ? (
                      <img
                        src={evidenceApi.getImageUrl(detail.primaryPhotoPath)}
                        alt={detail.name}
                        className="h-full w-full object-cover"
                      />
                    ) : (
                      <div className="flex h-full w-full items-center justify-center text-xs text-slate-500">
                        {t('card.noPhoto')}
                      </div>
                    )}
                  </div>
                  <div>
                    <div className="flex flex-wrap items-center gap-2">
                      <h3 className="text-lg font-bold text-[var(--text-primary)]">
                        {detail.name}
                      </h3>
                      {(() => {
                        const primary = detail.faces.find((f) => f.isPrimary) || detail.faces[0]
                        if (!primary) return null
                        const grade = getQualityGrade(primary.qualityScore, t)
                        return (
                          <span
                            className={`inline-flex items-center gap-1 rounded-md border ${grade.borderClass} ${grade.bgClass} px-2 py-0.5 text-[10px] font-medium ${grade.colorClass}`}
                          >
                            <span className={`h-1.5 w-1.5 rounded-full ${grade.barClass}`} />
                            {t('quality.sampleHealth')}: {grade.label} ({grade.scorePercent}%)
                          </span>
                        )
                      })()}
                    </div>
                    <p className="font-mono text-xs text-[var(--text-muted)]">
                      ID: {detail.subjectId}
                    </p>
                    <div className="mt-1 flex flex-wrap gap-2 text-[11px] text-[var(--text-secondary)]">
                      {detail.idCard && (
                        <span className="inline-flex items-center gap-1">
                          <IdCardIcon className="h-3 w-3 text-slate-400" />
                          {detail.idCard}
                        </span>
                      )}
                      {detail.remark && (
                        <span className="inline-flex items-center gap-1">
                          <Tag className="h-3 w-3 text-slate-400" />
                          {detail.remark}
                        </span>
                      )}
                    </div>
                  </div>
                </div>

                <div className="mt-3 flex items-center justify-between border-t border-[var(--border)] pt-2.5 text-[10px] text-[var(--text-muted)]">
                  <span className="flex items-center gap-1">
                    <Clock className="h-3 w-3" />
                    {t('card.registeredAt')}: {formatTimestamp(detail.createdAt)}
                  </span>
                </div>
              </div>

              {/* 人脸样本切片区 */}
              <div>
                <div className="mb-3 flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <h4 className="text-xs font-semibold tracking-wider text-[var(--text-secondary)] uppercase">
                      {t('modal.photoUploadTitle')} ({detail.faces.length}/5)
                    </h4>
                    {/* 切片视角切换 */}
                    <div className="flex rounded-lg border border-[var(--border)] bg-[var(--bg-tertiary)] p-0.5 text-[10px]">
                      <button
                        type="button"
                        onClick={() => setActiveTab('original')}
                        className={`rounded-md px-2 py-0.5 transition-colors ${
                          activeTab === 'original'
                            ? 'bg-emerald-500 font-semibold text-black'
                            : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                        }`}
                      >
                        {t('drawer.originalTab')}
                      </button>
                      <button
                        type="button"
                        onClick={() => setActiveTab('aligned')}
                        className={`rounded-md px-2 py-0.5 transition-colors ${
                          activeTab === 'aligned'
                            ? 'bg-emerald-500 font-semibold text-black'
                            : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                        }`}
                      >
                        {t('drawer.alignedTab')}
                      </button>
                    </div>
                  </div>

                  {detail.faces.length < 5 && (
                    <button
                      type="button"
                      onClick={() => fileInputRef.current?.click()}
                      disabled={actionLoading}
                      className="inline-flex items-center gap-1.5 rounded-lg border border-emerald-500/40 bg-emerald-500/10 px-2.5 py-1 text-xs font-medium text-emerald-400 transition-colors hover:bg-emerald-500/20"
                    >
                      <UploadCloud className="h-3.5 w-3.5" />
                      {t('actions.addFaces')}
                    </button>
                  )}
                  <input
                    ref={fileInputRef}
                    type="file"
                    accept="image/jpeg,image/png,image/webp"
                    multiple
                    className="hidden"
                    onChange={handleAddPhotos}
                  />
                </div>

                {/* 样本列表 */}
                <div className="grid grid-cols-2 gap-3.5">
                  {detail.faces.map((face) => {
                    const imgUrl =
                      activeTab === 'aligned' && face.alignedRelPath
                        ? evidenceApi.getImageUrl(face.alignedRelPath)
                        : evidenceApi.getImageUrl(face.photoRelPath)
                    const grade = getQualityGrade(face.qualityScore, t)

                    return (
                      <div
                        key={face.faceId}
                        className={`group relative flex flex-col overflow-hidden rounded-xl border p-2.5 shadow-xs transition-all ${
                          face.isPrimary
                            ? 'border-emerald-500/60 bg-emerald-500/5'
                            : 'border-[var(--border)] bg-[var(--bg-tertiary)]'
                        }`}
                      >
                        <div className="relative aspect-square w-full overflow-hidden rounded-lg bg-black/80">
                          <img
                            src={imgUrl}
                            alt={face.faceId}
                            className="h-full w-full object-cover"
                          />
                          {face.isPrimary && (
                            <div className="absolute top-1.5 left-1.5 flex items-center gap-1 rounded-md bg-emerald-500 px-1.5 py-0.5 text-[9px] font-bold text-black shadow-xs">
                              <Star className="h-2.5 w-2.5 fill-black" />
                              {t('card.primary')}
                            </div>
                          )}
                          <div
                            className={`absolute top-1.5 right-1.5 flex items-center gap-1 rounded-md border ${grade.borderClass} ${grade.bgClass} px-1.5 py-0.5 text-[9px] font-bold ${grade.colorClass} shadow-xs backdrop-blur-xs`}
                          >
                            <span className={`h-1.5 w-1.5 rounded-full ${grade.barClass}`} />
                            <span>{grade.label}</span>
                            <span>{grade.scorePercent}%</span>
                          </div>
                        </div>

                        {/* 质量分进度条与置信分指标 */}
                        <div className="mt-2.5 space-y-1">
                          <div className="flex items-center justify-between text-[10px]">
                            <span className="flex items-center gap-1 text-[var(--text-secondary)]">
                              <span className="text-[var(--text-muted)]">
                                {t('quality.scoreLabel')}:
                              </span>
                              <span className={`font-bold ${grade.colorClass}`}>
                                {grade.scorePercent}%
                              </span>
                            </span>
                            <span className="text-[10px] text-[var(--text-muted)]">
                              {t('quality.detectionLabel')}: {Math.round(face.detectionScore * 100)}
                              %
                            </span>
                          </div>
                          <div className="h-1.5 w-full overflow-hidden rounded-full bg-[var(--bg-primary)]">
                            <div
                              className={`h-full rounded-full transition-all duration-500 ${grade.barClass}`}
                              style={{
                                width: `${Math.min(100, Math.max(8, grade.scorePercent))}%`,
                              }}
                            />
                          </div>
                        </div>

                        {/* 操作栏 */}
                        <div className="mt-2.5 flex items-center justify-between border-t border-[var(--border)] pt-1.5 text-[11px]">
                          {!face.isPrimary ? (
                            <button
                              type="button"
                              onClick={() => handleSetPrimary(face)}
                              disabled={actionLoading}
                              className="text-[10px] text-[var(--text-secondary)] hover:text-emerald-400"
                            >
                              {t('actions.setPrimary')}
                            </button>
                          ) : (
                            <span className="flex items-center gap-1 text-[10px] text-emerald-400">
                              <CheckCircle2 className="h-3 w-3" />
                              {t('card.isPrimary')}
                            </span>
                          )}

                          <button
                            type="button"
                            onClick={() => handleDeleteFace(face)}
                            disabled={actionLoading || detail.faces.length <= 1}
                            className="p-1 text-[var(--text-muted)] transition-colors hover:text-red-400 disabled:opacity-30"
                            title={t('actions.deleteFace')}
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      </div>
                    )
                  })}
                </div>
              </div>
            </>
          ) : null}
        </div>
      </div>

      {/* 删除单张样本确认模态框 */}
      {faceToDelete && (
        <div className="animate-in fade-in fixed inset-0 z-60 flex items-center justify-center bg-black/70 p-4 backdrop-blur-xs duration-200">
          <div className="relative w-full max-w-sm overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-6 shadow-2xl">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-red-500/30 bg-red-500/10 text-red-400">
                <AlertCircle className="h-5 w-5" />
              </div>
              <h3 className="text-base font-semibold text-[var(--text-primary)]">
                {t('delete.deleteFaceTitle')}
              </h3>
            </div>
            <p className="mt-3 text-xs leading-relaxed text-[var(--text-secondary)]">
              {t('delete.deleteFaceDesc')}
            </p>
            <div className="mt-6 flex items-center justify-end gap-3">
              <button
                type="button"
                onClick={() => setFaceToDelete(null)}
                disabled={actionLoading}
                className="rounded-xl border border-[var(--border)] px-4 py-2 text-sm font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-tertiary)]"
              >
                {t('actions.cancel')}
              </button>
              <button
                type="button"
                onClick={handleConfirmDeleteFace}
                disabled={actionLoading}
                className="inline-flex items-center gap-2 rounded-xl bg-red-500 px-4 py-2 text-sm font-semibold text-white shadow-xs transition-colors hover:bg-red-600 disabled:opacity-50"
              >
                {actionLoading && <Loader2 className="h-4 w-4 animate-spin" />}
                {actionLoading ? t('actions.delete') : t('actions.confirm')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
