import React, { useState, useRef, useEffect, useCallback, useId } from 'react'
import {
  AlertCircle,
  Check,
  CheckCircle2,
  Clock,
  Copy,
  CreditCard,
  Eye,
  ImagePlus,
  Layers,
  Loader2,
  RefreshCw,
  Sparkles,
  Tag,
  Trash2,
  UploadCloud,
  UserCheck,
  X,
} from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { evidenceApi, personnelApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import { formatTimestamp } from '@/lib/time'
import { copyToClipboard } from '@/lib/utils'
import type { PersonnelDetail, GalleryFace, ReextractFaceFeaturesReport } from '@/types'
import { ReextractModal } from './ReextractModal'

export interface PersonnelDetailDrawerProps {
  isOpen: boolean
  subjectId: string | null
  autoOpenUpload?: boolean
  onClose: () => void
  onUpdate: () => void
  /** 操作成功后的即时反馈，由页面容器翻译成 Toast */
  onNotify?: (payload: { title: string; message: string }) => void
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
      colorClass: 'text-[var(--status-success)]',
      bgClass: 'bg-[var(--status-success-soft)]',
      borderClass: 'border-[var(--status-success-border)]',
      barClass: 'bg-[var(--status-success)]',
      scorePercent,
    }
  }
  if (scorePercent >= 65) {
    return {
      label: t('quality.good'),
      colorClass: 'text-[var(--status-info)]',
      bgClass: 'bg-[var(--status-info-soft)]',
      borderClass: 'border-[var(--status-info-border)]',
      barClass: 'bg-[var(--status-info)]',
      scorePercent,
    }
  }
  return {
    label: t('quality.fair'),
    colorClass: 'text-[var(--status-warning)]',
    bgClass: 'bg-[var(--status-warning-soft)]',
    borderClass: 'border-[var(--status-warning-border)]',
    barClass: 'bg-[var(--status-warning)]',
    scorePercent,
  }
}

export function PersonnelDetailDrawer({
  isOpen,
  subjectId,
  autoOpenUpload,
  onClose,
  onUpdate,
  onNotify,
}: PersonnelDetailDrawerProps): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])
  const reduceMotion = useReducedMotion()
  const titleId = useId()

  const [detail, setDetail] = useState<PersonnelDetail | null>(null)
  const [loading, setLoading] = useState(false)
  const [actionLoading, setActionLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [activeTab, setActiveTab] = useState<'original' | 'aligned'>('original')
  const [faceToDelete, setFaceToDelete] = useState<GalleryFace | null>(null)
  const [previewPhotoUrl, setPreviewPhotoUrl] = useState<string | null>(null)
  const [copiedId, setCopiedId] = useState(false)
  const copiedTimerRef = useRef<NodeJS.Timeout | null>(null)

  // 单人重新提取特征状态
  const [isReextractModalOpen, setIsReextractModalOpen] = useState(false)
  const [isReextracting, setIsReextracting] = useState(false)
  const [reextractReport, setReextractReport] = useState<ReextractFaceFeaturesReport | null>(null)
  const [reextractError, setReextractError] = useState<string | null>(null)

  const fileInputRef = useRef<HTMLInputElement>(null)

  // 1. 抽屉自身接入浮层栈（priority: 0）
  useDismissStack(Boolean(subjectId), onClose, { priority: 0 })

  // 2. 删除人脸样本子弹窗接入浮层栈（priority: 10），按 Esc 先关子弹窗
  useDismissStack(Boolean(faceToDelete), () => setFaceToDelete(null), {
    priority: 10,
    disabled: actionLoading,
  })

  // 3. 高清相片预览浮层接入浮层栈（priority: 20）
  useDismissStack(Boolean(previewPhotoUrl), () => setPreviewPhotoUrl(null), {
    priority: 20,
  })

  useEffect(() => {
    return () => {
      if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
    }
  }, [])

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
      setPreviewPhotoUrl(null)
    }
  }, [isOpen, subjectId, autoOpenUpload, fetchDetail])

  const handleCopyId = async (idText: string) => {
    const success = await copyToClipboard(idText)
    if (!success) return
    setCopiedId(true)
    if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
    copiedTimerRef.current = setTimeout(() => {
      setCopiedId(false)
    }, 1500)
  }

  const handleSetPrimary = async (face: GalleryFace) => {
    if (!detail || face.isPrimary) return
    setActionLoading(true)
    setError(null)
    try {
      const updated = await personnelApi.setPrimaryFace(detail.subjectId, face.faceId)
      setDetail(updated)
      onUpdate()
      onNotify?.({
        title: t('toast.primarySet'),
        message: `${updated.name} · ${updated.subjectId}`,
      })
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
      onNotify?.({
        title: t('toast.faceDeleted'),
        message: `${updated.name} · ${t('card.sampleCount')} ${updated.faces.length}/5`,
      })
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : t('errors.failedToDelete'))
    } finally {
      setActionLoading(false)
    }
  }

  const handleOpenReextract = () => {
    setReextractReport(null)
    setReextractError(null)
    setIsReextractModalOpen(true)
  }

  const handleConfirmReextract = async () => {
    if (!detail) return
    setIsReextracting(true)
    setReextractError(null)
    try {
      const report = await personnelApi.reextractSingle(detail.subjectId)
      setReextractReport(report)
      await fetchDetail(detail.subjectId)
      onUpdate()
    } catch (err: unknown) {
      setReextractError(err instanceof Error ? err.message : t('errors.reextractFailed'))
    } finally {
      setIsReextracting(false)
    }
  }

  const handleCloseReextractModal = () => {
    setIsReextractModalOpen(false)
    setReextractReport(null)
    setReextractError(null)
  }

  const handleAddPhotos = async (e: React.ChangeEvent<HTMLInputElement>) => {
    if (!detail) return
    const files = Array.from(e.target.files || [])
    if (!files.length) return

    const availableSlots = 5 - detail.faces.length
    if (files.length > availableSlots) {
      setError(t('errors.maxPhotosExceeded'))
      if (fileInputRef.current) {
        fileInputRef.current.value = ''
      }
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
      onNotify?.({
        title: t('toast.faceAdded'),
        message: `${updated.name} · ${t('card.sampleCount')} ${updated.faces.length}/5`,
      })
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : t('errors.failedToSave'))
    } finally {
      setActionLoading(false)
      if (fileInputRef.current) {
        fileInputRef.current.value = ''
      }
    }
  }

  const primaryFace = detail ? (detail.faces.find((f) => f.isPrimary) ?? detail.faces[0]) : null
  const primaryGrade = primaryFace ? getQualityGrade(primaryFace.qualityScore, t) : null
  const emptySlotsCount = detail ? Math.max(0, 5 - detail.faces.length) : 0

  return (
    <>
      <AnimatePresence>
        {isOpen && (
          <div className="fixed inset-0 z-50 flex justify-end">
            {/* 背景遮罩 */}
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: motionTokens.duration.fast }}
              onClick={onClose}
              className="fixed inset-0 bg-black/60 backdrop-blur-xs"
              aria-hidden="true"
            />

            {/* 抽屉主面板 */}
            <motion.aside
              role="dialog"
              aria-modal="true"
              aria-labelledby={titleId}
              initial={reduceMotion ? false : { x: '100%' }}
              animate={{ x: 0 }}
              exit={reduceMotion ? { opacity: 0 } : { x: '100%' }}
              transition={{
                duration: motionTokens.duration.normal,
                ease: motionTokens.easing.smooth,
              }}
              className="relative z-10 flex h-full w-full max-w-xl flex-col border-l border-[var(--border)] bg-[var(--bg-surface-solid)] shadow-2xl"
            >
              {/* ── 1. 抽屉 Header ── */}
              <header className="flex shrink-0 items-center justify-between border-b border-[var(--border)]/70 px-5 py-3.5 sm:px-6">
                <div className="flex min-w-0 items-center gap-3">
                  <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-[var(--accent)]/20 bg-[var(--accent-soft)] text-[var(--accent)] shadow-xs">
                    <UserCheck className="h-5 w-5" aria-hidden="true" />
                  </div>
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <h2
                        id={titleId}
                        className="truncate text-base font-bold tracking-tight text-[var(--text-primary)]"
                      >
                        {detail ? detail.name : t('actions.viewDetails')}
                      </h2>

                      {/* 工号胶囊（支持一键复制，绝不折断） */}
                      {detail && (
                        <button
                          type="button"
                          onClick={() => handleCopyId(detail.subjectId)}
                          title={t('common:copy', { defaultValue: '复制工号' })}
                          className="font-data group/id inline-flex items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--bg-secondary)]/80 px-1.5 py-0.5 text-[11px] font-semibold whitespace-nowrap text-[var(--accent)] transition-all hover:border-[var(--accent)] active:scale-95"
                        >
                          <span>{`#${detail.subjectId}`}</span>
                          {copiedId ? (
                            <Check className="h-2.5 w-2.5 text-[var(--status-success)]" />
                          ) : (
                            <Copy className="h-2.5 w-2.5 opacity-60 group-hover/id:opacity-100" />
                          )}
                        </button>
                      )}
                    </div>

                    <p className="mt-0.5 truncate text-xs text-[var(--text-muted)]">
                      {detail
                        ? detail.remark || t('card.noRemark')
                        : loading
                          ? t('common:loading')
                          : t('card.noPhoto')}
                    </p>
                  </div>
                </div>

                <div className="flex shrink-0 items-center gap-2.5">
                  {detail && (
                    <div
                      className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-[10px] font-semibold transition-colors ${
                        detail.faces.length === 5
                          ? 'border-[var(--status-success-border)] bg-[var(--status-success-soft)] text-[var(--status-success)]'
                          : 'border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] text-[var(--status-warning)]'
                      }`}
                    >
                      <div className="flex items-center gap-0.5">
                        {[1, 2, 3, 4, 5].map((slot) => {
                          let dotColor = 'bg-[var(--border-strong)]/40'
                          if (slot <= detail.faces.length) {
                            dotColor =
                              detail.faces.length === 5
                                ? 'bg-[var(--status-success)] shadow-[0_0_3px_var(--status-success-soft)]'
                                : 'bg-[var(--status-warning)]'
                          }
                          return (
                            <span key={slot} className={`h-1.5 w-1.5 rounded-xs ${dotColor}`} />
                          )
                        })}
                      </div>
                      <span className="font-data tabular-nums">{detail.faces.length}/5</span>
                    </div>
                  )}

                  <button
                    type="button"
                    onClick={onClose}
                    aria-label={t('common:close')}
                    className="flex h-8 w-8 items-center justify-center rounded-xl text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                  >
                    <X className="h-4 w-4" />
                  </button>
                </div>
              </header>

              {/* ── 2. 抽屉主体滚动力场 ── */}
              <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-5 sm:p-6">
                {loading ? (
                  <div className="flex h-72 flex-col items-center justify-center gap-3 text-[var(--text-muted)]">
                    <Loader2 className="h-7 w-7 animate-spin text-[var(--accent)]" />
                    <span className="text-xs">{t('common:loading')}</span>
                  </div>
                ) : detail ? (
                  <>
                    {/* 错误横幅提示 */}
                    {error && (
                      <div
                        role="alert"
                        className="flex items-start gap-2.5 rounded-2xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] p-3.5 text-xs text-[var(--status-danger)] shadow-2xs"
                      >
                        <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
                        <span className="leading-relaxed">{error}</span>
                      </div>
                    )}

                    {/* ── 区块 A：核心身份通行卡 (Identity Passport) ── */}
                    <div className="frosted-glass relative overflow-hidden rounded-2xl border border-[var(--border)] p-4 shadow-xs">
                      <div className="flex items-start gap-4">
                        {/* 独立肖像相框（支持点击无损大图预览） */}
                        <div
                          role="button"
                          tabIndex={0}
                          onClick={() => {
                            if (detail.primaryPhotoPath) {
                              setPreviewPhotoUrl(evidenceApi.getImageUrl(detail.primaryPhotoPath))
                            }
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter' || e.key === ' ') {
                              e.preventDefault()
                              if (detail.primaryPhotoPath) {
                                setPreviewPhotoUrl(evidenceApi.getImageUrl(detail.primaryPhotoPath))
                              }
                            }
                          }}
                          aria-label={t('table.previewPhoto', { defaultValue: '查看主照片' })}
                          title={t('table.previewPhoto', { defaultValue: '查看主照片' })}
                          className="group/avatar relative h-28 w-22 shrink-0 cursor-pointer overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] shadow-inner sm:h-32 sm:w-26"
                        >
                          {detail.primaryPhotoPath ? (
                            <>
                              <img
                                src={evidenceApi.getImageUrl(detail.primaryPhotoPath)}
                                alt={detail.name}
                                className="h-full w-full object-cover transition-transform duration-300 group-hover/avatar:scale-105"
                              />
                              <div className="absolute inset-0 flex items-center justify-center bg-black/35 opacity-0 transition-opacity group-hover/avatar:opacity-100">
                                <Eye className="h-4 w-4 text-white drop-shadow-md" />
                              </div>
                            </>
                          ) : (
                            <div className="flex h-full w-full flex-col items-center justify-center p-2 text-center text-xs text-[var(--text-muted)]">
                              <UserCheck className="h-6 w-6 opacity-40" />
                              <span className="mt-1 text-[10px]">{t('card.noPhoto')}</span>
                            </div>
                          )}

                          {/* 主头像微标 */}
                          <div className="absolute bottom-1.5 left-1.5 z-10">
                            <span className="inline-flex items-center gap-0.5 rounded-md border border-[var(--status-success-border)] bg-[var(--overlay-scrim)] px-1.5 py-0.5 text-[9px] font-semibold text-[var(--status-success)] shadow-xs backdrop-blur-md">
                              <Sparkles className="h-2.5 w-2.5 text-[var(--status-success)]" />
                              <span>{t('card.primary')}</span>
                            </span>
                          </div>
                        </div>

                        {/* 右侧属性清单 */}
                        <div className="min-w-0 flex-1 space-y-2">
                          <div>
                            <h3 className="truncate text-lg font-bold tracking-tight text-[var(--text-primary)]">
                              {detail.name}
                            </h3>
                            <div className="mt-1 flex items-center gap-1.5 text-xs text-[var(--text-secondary)]">
                              <Tag className="h-3.5 w-3.5 shrink-0 text-[var(--text-muted)]" />
                              <span className="truncate">
                                {detail.remark || t('card.noRemark')}
                              </span>
                            </div>
                          </div>

                          <div className="space-y-1 text-xs text-[var(--text-secondary)]">
                            <div className="flex items-center gap-1.5">
                              <CreditCard className="h-3.5 w-3.5 shrink-0 text-[var(--text-muted)]" />
                              <span className="font-data truncate text-[11px] select-text">
                                {detail.idCard || t('card.noIdCard')}
                              </span>
                            </div>
                            <div className="flex items-center gap-1.5 text-[11px] text-[var(--text-muted)]">
                              <Clock className="h-3.5 w-3.5 shrink-0" />
                              <span className="font-data whitespace-nowrap tabular-nums">
                                {formatTimestamp(detail.createdAt)}
                              </span>
                            </div>
                          </div>
                        </div>
                      </div>

                      {/* ── 主样本人脸质检度量 ── */}
                      {primaryGrade && (
                        <div className="mt-3.5 rounded-xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/50 p-2.5">
                          <div className="flex items-center justify-between text-xs">
                            <span className="flex items-center gap-1.5 font-medium text-[var(--text-secondary)]">
                              <Layers className="h-3.5 w-3.5 text-[var(--accent)]" />
                              <span>{t('quality.sampleHealth')}</span>
                            </span>
                            <span
                              className={`font-data inline-flex items-center gap-1 font-semibold ${primaryGrade.colorClass}`}
                            >
                              <span
                                aria-hidden="true"
                                className={`h-1.5 w-1.5 rounded-full ${primaryGrade.barClass}`}
                              />
                              <span>{primaryGrade.label}</span>
                              <span className="tabular-nums">({primaryGrade.scorePercent}%)</span>
                            </span>
                          </div>
                          <div className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-[var(--bg-secondary)]">
                            <div
                              className={`h-full rounded-full transition-all duration-500 ${primaryGrade.barClass}`}
                              style={{ width: `${Math.max(6, primaryGrade.scorePercent)}%` }}
                            />
                          </div>
                        </div>
                      )}
                    </div>

                    {/* ── 区块 B：多视角人脸样本库 (Multi-Angle Gallery) ── */}
                    <div className="rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)]/30 p-4">
                      {/* 顶栏：标题 + 原始/切片 Tab 切换 */}
                      <div className="flex flex-wrap items-center justify-between gap-2.5">
                        <div className="flex items-center gap-2">
                          <h3 className="text-xs font-bold tracking-wider text-[var(--text-primary)] uppercase">
                            {t('modal.photoUploadTitle')}
                          </h3>
                          <span className="font-data rounded-full border border-[var(--border)] bg-[var(--bg-surface)] px-2 py-0.5 text-[10px] font-semibold text-[var(--text-secondary)] tabular-nums">
                            {detail.faces.length}/5
                          </span>
                        </div>

                        {/* 切片视角切换 (iOS/SaaS 风格 Segmented Controls) */}
                        <div
                          role="group"
                          aria-label={t('modal.photoUploadTitle')}
                          className="flex rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/80 p-0.5 text-[11px]"
                        >
                          <button
                            type="button"
                            onClick={() => setActiveTab('original')}
                            aria-pressed={activeTab === 'original'}
                            className={`rounded-lg px-2.5 py-1 font-medium transition-all ${
                              activeTab === 'original'
                                ? 'bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs'
                                : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                            }`}
                          >
                            {t('drawer.originalTab')}
                          </button>
                          <button
                            type="button"
                            onClick={() => setActiveTab('aligned')}
                            aria-pressed={activeTab === 'aligned'}
                            className={`rounded-lg px-2.5 py-1 font-medium transition-all ${
                              activeTab === 'aligned'
                                ? 'bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs'
                                : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                            }`}
                          >
                            {t('drawer.alignedTab')}
                          </button>
                        </div>
                      </div>

                      {/* 动作栏：重新提取 + 追加照片 */}
                      <div className="mt-3.5 flex flex-wrap items-center justify-between gap-2">
                        <button
                          type="button"
                          onClick={handleOpenReextract}
                          disabled={actionLoading || isReextracting || detail.faces.length === 0}
                          title={t('actions.reextractShort')}
                          className="inline-flex h-8 items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 text-xs font-medium text-[var(--text-secondary)] transition-all hover:border-[var(--border-strong)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-40"
                        >
                          <RefreshCw
                            className={`h-3.5 w-3.5 ${isReextracting ? 'animate-spin text-[var(--accent)]' : ''}`}
                            aria-hidden="true"
                          />
                          <span>{t('actions.reextractShort')}</span>
                        </button>

                        {detail.faces.length < 5 && (
                          <button
                            type="button"
                            onClick={() => fileInputRef.current?.click()}
                            disabled={actionLoading || isReextracting}
                            className="inline-flex h-8 items-center gap-1.5 rounded-xl bg-[var(--accent)] px-3.5 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none active:scale-95 disabled:opacity-40"
                          >
                            <UploadCloud className="h-3.5 w-3.5" aria-hidden="true" />
                            <span>{t('actions.addFaces')}</span>
                          </button>
                        )}

                        <input
                          ref={fileInputRef}
                          type="file"
                          accept="image/jpeg,image/png,image/webp"
                          multiple
                          className="hidden"
                          aria-label={t('actions.addFaces')}
                          onChange={handleAddPhotos}
                        />
                      </div>

                      {/* ── 样本网格卡片 ── */}
                      <div className="mt-3.5 grid grid-cols-2 gap-3 sm:grid-cols-2">
                        {detail.faces.map((face) => {
                          const imgUrl =
                            activeTab === 'aligned' && face.alignedRelPath
                              ? evidenceApi.getImageUrl(face.alignedRelPath)
                              : evidenceApi.getImageUrl(face.photoRelPath)
                          const grade = getQualityGrade(face.qualityScore, t)

                          return (
                            <div
                              key={face.faceId}
                              className={`group/tile relative flex flex-col justify-between rounded-2xl border p-2.5 shadow-2xs transition-all hover:shadow-md ${
                                face.isPrimary
                                  ? 'border-[var(--status-success-border)] bg-[var(--status-success-soft)]'
                                  : 'border-[var(--border)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]'
                              }`}
                            >
                              {/* 相片视窗（支持点击大图预览） */}
                              <div
                                role="button"
                                tabIndex={0}
                                onClick={() => setPreviewPhotoUrl(imgUrl)}
                                onKeyDown={(e) => {
                                  if (e.key === 'Enter' || e.key === ' ') {
                                    e.preventDefault()
                                    setPreviewPhotoUrl(imgUrl)
                                  }
                                }}
                                title={t('table.previewPhoto', { defaultValue: '查看大图' })}
                                aria-label={t('table.previewPhoto', { defaultValue: '查看大图' })}
                                className="relative aspect-4/3 w-full cursor-pointer overflow-hidden rounded-xl bg-[var(--video-surface)] sm:aspect-square"
                              >
                                <img
                                  src={imgUrl}
                                  alt={face.faceId}
                                  loading="lazy"
                                  className="h-full w-full object-cover transition-transform duration-300 group-hover/tile:scale-105"
                                />

                                {/* 悬停放大镜遮罩 */}
                                <div className="absolute inset-0 flex items-center justify-center bg-[var(--overlay-scrim)] opacity-0 transition-opacity group-hover/tile:opacity-100">
                                  <Eye className="h-4 w-4 text-white drop-shadow" />
                                </div>

                                {/* 主头像徽标 */}
                                {face.isPrimary && (
                                  <div className="absolute top-1.5 left-1.5 z-10 flex items-center gap-1 rounded-md bg-[var(--status-success)] px-1.5 py-0.5 text-[9px] font-bold text-white shadow-xs">
                                    <Sparkles className="h-2.5 w-2.5" />
                                    <span>{t('card.primary')}</span>
                                  </div>
                                )}

                                {/* 质量分角标 */}
                                <div
                                  className={`absolute top-1.5 right-1.5 z-10 flex items-center gap-1 rounded-md border ${grade.borderClass} ${grade.bgClass} px-1.5 py-0.5 text-[9px] font-bold ${grade.colorClass} shadow-xs backdrop-blur-md`}
                                >
                                  <span
                                    aria-hidden="true"
                                    className={`h-1.5 w-1.5 rounded-full ${grade.barClass}`}
                                  />
                                  <span>{grade.label}</span>
                                  <span className="font-data tabular-nums">
                                    {grade.scorePercent}%
                                  </span>
                                </div>
                              </div>

                              {/* 质量分进度条与置信度 */}
                              <div className="mt-2 space-y-1">
                                <div className="flex items-center justify-between text-[10px]">
                                  <span className="text-[var(--text-muted)]">
                                    {t('quality.scoreLabel')}:{' '}
                                    <strong className={grade.colorClass}>
                                      {grade.scorePercent}%
                                    </strong>
                                  </span>
                                  <span className="font-data text-[var(--text-muted)] tabular-nums">
                                    {t('quality.detectionLabel')}:{' '}
                                    {Math.round(face.detectionScore * 100)}%
                                  </span>
                                </div>
                                <div className="h-1 w-full overflow-hidden rounded-full bg-[var(--bg-secondary)]">
                                  <div
                                    className={`h-full rounded-full transition-all duration-300 ${grade.barClass}`}
                                    style={{ width: `${Math.max(6, grade.scorePercent)}%` }}
                                  />
                                </div>
                              </div>

                              {/* 底栏动作控制 */}
                              <div className="mt-2.5 flex items-center justify-between border-t border-[var(--border)]/70 pt-1.5">
                                {!face.isPrimary ? (
                                  <button
                                    type="button"
                                    onClick={() => handleSetPrimary(face)}
                                    disabled={actionLoading}
                                    className="rounded-md px-1.5 py-0.5 text-[11px] font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-40"
                                  >
                                    {t('actions.setPrimary')}
                                  </button>
                                ) : (
                                  <span className="inline-flex items-center gap-1 px-1 text-[10px] font-semibold text-[var(--status-success)]">
                                    <CheckCircle2 className="h-3 w-3" aria-hidden="true" />
                                    <span>{t('card.isPrimary')}</span>
                                  </span>
                                )}

                                <button
                                  type="button"
                                  onClick={() => handleDeleteFace(face)}
                                  disabled={actionLoading || detail.faces.length <= 1}
                                  aria-label={t('actions.deleteFace')}
                                  title={t('actions.deleteFace')}
                                  className="flex h-7 w-7 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--status-danger-soft)] hover:text-[var(--status-danger)] focus-visible:ring-2 focus-visible:ring-[var(--status-danger)]/50 focus-visible:outline-none disabled:opacity-30"
                                >
                                  <Trash2 className="h-3.5 w-3.5" />
                                </button>
                              </div>
                            </div>
                          )
                        })}

                        {/* ── 空位追加虚线卡片 (Empty Slot Tiles) ── */}
                        {Array.from({ length: emptySlotsCount }).map((_, idx) => {
                          const slotIndex = detail.faces.length + idx + 1
                          return (
                            <button
                              key={`empty-slot-${slotIndex}`}
                              type="button"
                              onClick={() => fileInputRef.current?.click()}
                              disabled={actionLoading || isReextracting}
                              className="group/empty flex aspect-4/3 flex-col items-center justify-center rounded-2xl border-2 border-dashed border-[var(--border-strong)]/70 bg-[var(--bg-secondary)]/20 p-4 text-center transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)]/20 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-40 sm:aspect-square"
                            >
                              <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-muted)] shadow-2xs transition-transform duration-200 group-hover/empty:scale-110 group-hover/empty:border-[var(--accent)]/40 group-hover/empty:text-[var(--accent)]">
                                <ImagePlus className="h-5 w-5" />
                              </div>
                              <span className="mt-2 text-xs font-semibold text-[var(--text-secondary)] group-hover/empty:text-[var(--text-primary)]">
                                {t('actions.addFacesShort', { defaultValue: '追加样本' })}
                              </span>
                              <span className="font-data mt-0.5 text-[10px] text-[var(--text-muted)]">
                                Slot {slotIndex}/5
                              </span>
                            </button>
                          )
                        })}
                      </div>
                    </div>
                  </>
                ) : (
                  !error && (
                    <div className="flex h-72 flex-col items-center justify-center gap-3 text-[var(--text-muted)]">
                      <Loader2 className="h-7 w-7 animate-spin text-[var(--accent)]" />
                    </div>
                  )
                )}

                {!loading && !detail && error && (
                  <div
                    role="alert"
                    className="flex items-start gap-2.5 rounded-2xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] p-4 text-xs text-[var(--status-danger)]"
                  >
                    <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
                    <div className="flex-1 space-y-2">
                      <p className="leading-relaxed">{error}</p>
                      {subjectId && (
                        <button
                          type="button"
                          onClick={() => fetchDetail(subjectId)}
                          className="inline-flex h-8 items-center gap-1.5 rounded-xl border border-[var(--status-danger-border)] px-3 text-xs font-medium transition-colors hover:bg-[var(--status-danger-soft)] focus-visible:ring-2 focus-visible:ring-[var(--status-danger)]/40 focus-visible:outline-none"
                        >
                          <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
                          <span>{t('actions.retry')}</span>
                        </button>
                      )}
                    </div>
                  </div>
                )}
              </div>
            </motion.aside>
          </div>
        )}
      </AnimatePresence>

      {/* ── 全尺寸高清原图预览 Lightbox ── */}
      <AnimatePresence>
        {previewPhotoUrl && (
          <div
            onClick={() => setPreviewPhotoUrl(null)}
            className="fixed inset-0 z-[70] flex items-center justify-center bg-[var(--overlay-scrim)] p-4 backdrop-blur-md"
          >
            <motion.div
              initial={reduceMotion ? false : { opacity: 0, scale: 0.94 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={reduceMotion ? { opacity: 0 } : { opacity: 0, scale: 0.94 }}
              transition={{ duration: motionTokens.duration.fast }}
              onClick={(e) => e.stopPropagation()}
              className="relative max-h-[85vh] max-w-[85vw] overflow-hidden rounded-2xl border border-white/20 bg-[var(--video-surface)] shadow-2xl"
            >
              <img
                src={previewPhotoUrl}
                alt="Enlarged portrait"
                className="max-h-[80vh] max-w-[80vw] object-contain"
              />
              <button
                type="button"
                onClick={() => setPreviewPhotoUrl(null)}
                aria-label={t('common:close')}
                className="absolute top-3 right-3 flex h-8 w-8 items-center justify-center rounded-full bg-[var(--overlay-scrim)] text-white transition-colors hover:bg-black/90 focus-visible:ring-2 focus-visible:ring-white focus-visible:outline-none"
              >
                <X className="h-4 w-4" />
              </button>
            </motion.div>
          </div>
        )}
      </AnimatePresence>

      {/* ── 删除单张样本确认弹窗 ── */}
      <AnimatePresence>
        {faceToDelete && (
          <div
            onClick={(e) => {
              if (e.target === e.currentTarget && !actionLoading) setFaceToDelete(null)
            }}
            className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
          >
            <motion.div
              role="alertdialog"
              aria-modal="true"
              initial={reduceMotion ? false : { opacity: 0, scale: 0.96, y: 10 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={reduceMotion ? { opacity: 0 } : { opacity: 0, scale: 0.96, y: 10 }}
              transition={{
                duration: motionTokens.duration.fast,
                ease: motionTokens.easing.smooth,
              }}
              className="relative w-full max-w-sm overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-6 shadow-2xl"
            >
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)]">
                  <AlertCircle className="h-5 w-5" aria-hidden="true" />
                </div>
                <h3 className="text-base font-bold tracking-tight text-[var(--text-primary)]">
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
                  className="inline-flex h-9 items-center justify-center rounded-xl border border-[var(--border)] px-4 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
                >
                  {t('actions.cancel')}
                </button>
                <button
                  type="button"
                  onClick={handleConfirmDeleteFace}
                  disabled={actionLoading}
                  className="inline-flex h-9 items-center gap-2 rounded-xl bg-[var(--status-danger)] px-4 text-xs font-semibold text-white shadow-xs transition-colors hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--status-danger)]/50 focus-visible:outline-none disabled:opacity-50"
                >
                  {actionLoading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                  <span>{actionLoading ? t('actions.delete') : t('actions.confirm')}</span>
                </button>
              </div>
            </motion.div>
          </div>
        )}
      </AnimatePresence>

      {/* ── 单人重新提取特征弹窗 ── */}
      <ReextractModal
        isOpen={isReextractModalOpen}
        isGlobal={false}
        targetName={detail?.name}
        progress={null}
        singleReport={reextractReport}
        isStarting={isReextracting}
        error={reextractError}
        onClose={handleCloseReextractModal}
        onConfirm={handleConfirmReextract}
      />
    </>
  )
}
