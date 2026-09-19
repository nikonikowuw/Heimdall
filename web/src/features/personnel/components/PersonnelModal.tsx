import React, { useState, useRef, useEffect, useCallback, useId } from 'react'
import { AlertCircle, Image as ImageIcon, Loader2, Star, UploadCloud, X } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { personnelApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import type { PersonnelItem, PersonnelDetail } from '@/types'

export interface PersonnelModalProps {
  isOpen: boolean
  onClose: () => void
  onSuccess: (detail: PersonnelDetail) => void
  editTarget?: PersonnelItem | null
  onManagePhotos?: (person: PersonnelItem) => void
}

interface ImageFilePreview {
  file: File
  previewUrl: string
}

const MAX_PHOTOS = 5

const FIELD_CLASS =
  'w-full rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/70 px-3 py-2 text-sm text-[var(--text-primary)] transition-colors placeholder:text-[var(--text-muted)] hover:border-[var(--border-strong)] focus:border-emerald-500/70 focus:bg-[var(--bg-surface)] focus:ring-2 focus:ring-emerald-500/15 focus:outline-none disabled:cursor-not-allowed disabled:opacity-60'

const LABEL_CLASS = 'mb-1.5 block text-xs font-medium text-[var(--text-secondary)]'

export function PersonnelModal({
  isOpen,
  onClose,
  onSuccess,
  editTarget,
  onManagePhotos,
}: PersonnelModalProps): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])
  const reduceMotion = useReducedMotion()
  const isEdit = Boolean(editTarget)

  const [name, setName] = useState('')
  const [subjectId, setSubjectId] = useState('')
  const [idCard, setIdCard] = useState('')
  const [remark, setRemark] = useState('')
  const [selectedImages, setSelectedImages] = useState<ImageFilePreview[]>([])
  const [primaryIndex, setPrimaryIndex] = useState(0)

  const [isSubmitting, setIsSubmitting] = useState(false)
  const [isDragging, setIsDragging] = useState(false)
  const [errorMessage, setErrorMessage] = useState<string | null>(null)

  const titleId = useId()
  const nameId = useId()
  const subjectIdFieldId = useId()
  const idCardFieldId = useId()
  const remarkFieldId = useId()

  const fileInputRef = useRef<HTMLInputElement>(null)
  const previewsRef = useRef<ImageFilePreview[]>([])
  previewsRef.current = selectedImages

  const cleanupPreviews = useCallback(() => {
    previewsRef.current.forEach((img) => {
      URL.revokeObjectURL(img.previewUrl)
    })
  }, [])

  useEffect(() => {
    return () => {
      cleanupPreviews()
    }
  }, [cleanupPreviews])

  const handleClose = () => {
    onClose()
  }

  // 打开时重置表单状态；关闭后的清理交给退出动画结束回调，
  // 避免面板还在淡出时预览图先被回收而闪出碎图
  useEffect(() => {
    if (!isOpen) return
    setErrorMessage(null)
    setIsDragging(false)
    cleanupPreviews()
    if (editTarget) {
      setName(editTarget.name)
      setSubjectId(editTarget.subjectId)
      setIdCard(editTarget.idCard)
      setRemark(editTarget.remark)
      setSelectedImages([])
    } else {
      setName('')
      setSubjectId('')
      setIdCard('')
      setRemark('')
      setSelectedImages([])
      setPrimaryIndex(0)
    }
  }, [isOpen, editTarget, cleanupPreviews])

  const handleExitComplete = () => {
    cleanupPreviews()
    setSelectedImages([])
    setErrorMessage(null)
  }

  useDismissStack(isOpen, handleClose, { disabled: isSubmitting })

  const appendFiles = (files: File[]) => {
    if (!files.length) return
    setErrorMessage(null)

    const availableSlots = MAX_PHOTOS - selectedImages.length
    if (availableSlots <= 0) {
      setErrorMessage(t('errors.maxPhotosExceeded'))
      return
    }

    const toAdd = files.slice(0, availableSlots)
    if (files.length > availableSlots) {
      setErrorMessage(t('errors.maxPhotosExceeded'))
    }

    const newPreviews: ImageFilePreview[] = toAdd.map((file) => ({
      file,
      previewUrl: URL.createObjectURL(file),
    }))

    setSelectedImages((prev) => [...prev, ...newPreviews])
  }

  const handleImageSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    appendFiles(Array.from(e.target.files || []))
    if (fileInputRef.current) {
      fileInputRef.current.value = ''
    }
  }

  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault()
    if (isSubmitting) return
    setIsDragging(true)
  }

  const handleDragLeave = (e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(false)
  }

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(false)
    if (isSubmitting) return
    const dropped = Array.from(e.dataTransfer.files).filter((file) =>
      file.type.startsWith('image/'),
    )
    if (dropped.length === 0) return
    appendFiles(dropped)
  }

  const handleRemoveImage = (index: number) => {
    setSelectedImages((prev) => {
      const target = prev[index]
      if (target) {
        URL.revokeObjectURL(target.previewUrl)
      }
      const updated = prev.filter((_, i) => i !== index)
      if (primaryIndex >= updated.length) {
        setPrimaryIndex(Math.max(0, updated.length - 1))
      }
      return updated
    })
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setErrorMessage(null)

    if (!name.trim()) {
      setErrorMessage(t('errors.nameRequired'))
      return
    }

    if (!isEdit && selectedImages.length === 0) {
      setErrorMessage(t('errors.photoRequired'))
      return
    }

    setIsSubmitting(true)
    try {
      if (isEdit && editTarget) {
        const updated = await personnelApi.update(editTarget.subjectId, {
          name: name.trim(),
          idCard: idCard.trim(),
          remark: remark.trim(),
        })
        onSuccess(updated)
        onClose()
      } else {
        const formData = new FormData()
        formData.append('name', name.trim())
        if (subjectId.trim()) {
          formData.append('subjectId', subjectId.trim())
        }
        if (idCard.trim()) {
          formData.append('idCard', idCard.trim())
        }
        if (remark.trim()) {
          formData.append('remark', remark.trim())
        }

        // 把主头像放在第一位发送
        const ordered = [...selectedImages]
        if (primaryIndex > 0 && primaryIndex < ordered.length) {
          const [primaryItem] = ordered.splice(primaryIndex, 1)
          ordered.unshift(primaryItem)
        }

        for (const img of ordered) {
          formData.append('images', img.file)
        }

        const created = await personnelApi.create(formData)
        onSuccess(created)
        onClose()
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : t('errors.failedToSave')
      setErrorMessage(msg)
    } finally {
      setIsSubmitting(false)
    }
  }

  let submitButtonText = isEdit ? t('actions.save') : t('actions.confirm')
  if (isSubmitting) {
    submitButtonText = isEdit ? t('actions.saving') : t('actions.uploading')
  }

  return (
    <AnimatePresence onExitComplete={handleExitComplete}>
      {isOpen && (
        <div
          onClick={(e) => {
            if (e.target === e.currentTarget && !isSubmitting) {
              handleClose()
            }
          }}
          className="fixed inset-0 z-50 flex items-center justify-center bg-[var(--overlay-scrim)] p-4 backdrop-blur-sm"
        >
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-labelledby={titleId}
            initial={reduceMotion ? false : { opacity: 0, scale: 0.96, y: 12 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={reduceMotion ? { opacity: 0 } : { opacity: 0, scale: 0.96, y: 12 }}
            transition={{
              duration: motionTokens.duration.normal,
              ease: motionTokens.easing.smooth,
            }}
            className="relative flex max-h-[92vh] w-full max-w-xl flex-col overflow-hidden rounded-[26px] border border-[var(--border)] bg-[var(--bg-surface)] shadow-[0_28px_60px_-16px_rgba(0,0,0,0.4)] backdrop-blur-2xl"
          >
            {/* ── 1. 头部 ── */}
            <div className="flex shrink-0 items-center justify-between gap-3 border-b border-[var(--border)]/70 px-5 py-4 sm:px-6">
              <div className="flex min-w-0 items-center gap-3.5">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-emerald-500/20 bg-emerald-500/10 text-emerald-500 shadow-xs">
                  <ImageIcon className="h-5 w-5" aria-hidden="true" />
                </div>
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <h2
                      id={titleId}
                      className="truncate text-base font-bold tracking-tight text-[var(--text-primary)] sm:text-lg"
                    >
                      {isEdit ? t('modal.editTitle') : t('modal.registerTitle')}
                    </h2>
                    <span className="hidden rounded-full border border-[var(--border)]/70 bg-[var(--bg-secondary)]/70 px-2 py-0.5 text-[10px] font-semibold text-[var(--text-muted)] sm:inline-block">
                      {isEdit ? 'EDIT' : 'NEW'}
                    </span>
                  </div>
                  <p className="mt-0.5 truncate text-xs text-[var(--text-muted)]">
                    {isEdit && editTarget
                      ? `${editTarget.name} · ${editTarget.subjectId}`
                      : t('modal.subjectIdPlaceholder')}
                  </p>
                </div>
              </div>

              <button
                type="button"
                onClick={handleClose}
                disabled={isSubmitting}
                aria-label={t('common:close')}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* ── 2. 表单 ── */}
            <form onSubmit={handleSubmit} className="flex min-h-0 flex-1 flex-col">
              <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-5 sm:px-6">
                {errorMessage && (
                  <div
                    role="alert"
                    className="flex items-start gap-2.5 rounded-xl border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-500"
                  >
                    <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
                    <span className="leading-relaxed">{errorMessage}</span>
                  </div>
                )}

                <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                  <div>
                    <label htmlFor={nameId} className={LABEL_CLASS}>
                      {t('modal.name')} <span className="text-rose-500">*</span>
                    </label>
                    <input
                      id={nameId}
                      type="text"
                      value={name}
                      onChange={(e) => setName(e.target.value)}
                      placeholder={t('modal.namePlaceholder')}
                      className={FIELD_CLASS}
                      required
                    />
                  </div>

                  <div>
                    <label htmlFor={subjectIdFieldId} className={LABEL_CLASS}>
                      {t('modal.subjectId')}
                    </label>
                    <input
                      id={subjectIdFieldId}
                      type="text"
                      value={subjectId}
                      onChange={(e) => setSubjectId(e.target.value)}
                      placeholder={t('modal.subjectIdPlaceholder')}
                      disabled={isEdit}
                      className={`${FIELD_CLASS} font-data`}
                    />
                  </div>
                </div>

                <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                  <div>
                    <label htmlFor={idCardFieldId} className={LABEL_CLASS}>
                      {t('modal.idCard')}
                    </label>
                    <input
                      id={idCardFieldId}
                      type="text"
                      value={idCard}
                      onChange={(e) => setIdCard(e.target.value)}
                      placeholder={t('modal.idCardPlaceholder')}
                      className={FIELD_CLASS}
                    />
                  </div>

                  <div>
                    <label htmlFor={remarkFieldId} className={LABEL_CLASS}>
                      {t('modal.remark')}
                    </label>
                    <input
                      id={remarkFieldId}
                      type="text"
                      value={remark}
                      onChange={(e) => setRemark(e.target.value)}
                      placeholder={t('modal.remarkPlaceholder')}
                      className={FIELD_CLASS}
                    />
                  </div>
                </div>

                {/* 编辑模式下的人脸样本库入口提示与快捷追加 */}
                {isEdit && editTarget && (
                  <div className="flex flex-col gap-2.5 rounded-2xl border border-emerald-500/20 bg-emerald-500/5 p-3.5 sm:flex-row sm:items-center sm:justify-between">
                    <div className="flex min-w-0 items-start gap-2.5">
                      <ImageIcon
                        className="mt-0.5 h-4 w-4 shrink-0 text-emerald-500"
                        aria-hidden="true"
                      />
                      <div className="min-w-0">
                        <p className="text-xs font-semibold text-[var(--text-primary)]">
                          {t('modal.faceSampleManagement')} ({editTarget.faceCount}/{MAX_PHOTOS})
                        </p>
                        <p className="mt-0.5 text-[11px] leading-relaxed text-[var(--text-muted)]">
                          {t('modal.faceSampleManagementDesc')}
                        </p>
                      </div>
                    </div>
                    {onManagePhotos && (
                      <button
                        type="button"
                        onClick={() => {
                          handleClose()
                          onManagePhotos(editTarget)
                        }}
                        className="inline-flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-lg border border-emerald-500/40 bg-emerald-500/10 px-2.5 text-xs font-medium text-emerald-500 transition-colors hover:bg-emerald-500/20 focus-visible:ring-2 focus-visible:ring-emerald-500/40 focus-visible:outline-none"
                      >
                        <UploadCloud className="h-3.5 w-3.5" aria-hidden="true" />
                        {t('actions.manageOrAddFaces')}
                      </button>
                    )}
                  </div>
                )}

                {/* 注册模式下的多图上传区 */}
                {!isEdit && (
                  <div
                    onDragOver={handleDragOver}
                    onDragLeave={handleDragLeave}
                    onDrop={handleDrop}
                    className={`rounded-2xl border p-3 transition-colors ${
                      isDragging
                        ? 'border-emerald-500/60 bg-emerald-500/10'
                        : 'border-[var(--border)]/70 bg-[var(--bg-secondary)]/25'
                    }`}
                  >
                    <div className="mb-3 flex items-center justify-between gap-2">
                      <div>
                        <p className="text-xs font-medium text-[var(--text-secondary)]">
                          {t('modal.photoUploadTitle')} <span className="text-rose-500">*</span>
                        </p>
                        <p className="mt-0.5 text-[11px] text-[var(--text-muted)]">
                          {t('modal.photoUploadDesc')}
                        </p>
                      </div>
                      <span className="font-data shrink-0 rounded-full border border-[var(--border)]/70 bg-[var(--bg-surface)]/70 px-2 py-0.5 text-[10px] font-semibold text-[var(--text-secondary)] tabular-nums">
                        {t('modal.photoCounter', {
                          current: selectedImages.length,
                          max: MAX_PHOTOS,
                        })}
                      </span>
                    </div>

                    <div className="grid grid-cols-3 gap-2.5 sm:grid-cols-5">
                      {selectedImages.map((img, idx) => (
                        <div
                          key={img.previewUrl}
                          className={`group relative aspect-square overflow-hidden rounded-xl border bg-black/40 shadow-xs ${
                            primaryIndex === idx
                              ? 'border-emerald-500/70 ring-2 ring-emerald-500/25'
                              : 'border-[var(--border)]'
                          }`}
                        >
                          <img
                            src={img.previewUrl}
                            alt={`${t('modal.photoUploadTitle')} ${idx + 1}`}
                            className="h-full w-full object-cover"
                          />

                          {/* 主头像标识 / 切换按钮 */}
                          <button
                            type="button"
                            onClick={() => setPrimaryIndex(idx)}
                            aria-pressed={primaryIndex === idx}
                            aria-label={
                              primaryIndex === idx
                                ? t('modal.primaryBadge')
                                : t('modal.setAsPrimary')
                            }
                            title={
                              primaryIndex === idx
                                ? t('modal.primaryBadge')
                                : t('modal.setAsPrimary')
                            }
                            className={`absolute top-1 left-1 flex h-6 w-6 items-center justify-center rounded-md transition-colors focus-visible:ring-2 focus-visible:ring-emerald-500/60 focus-visible:outline-none ${
                              primaryIndex === idx
                                ? 'bg-emerald-500 text-black shadow-xs'
                                : 'bg-black/60 text-white/70 hover:text-amber-400'
                            }`}
                          >
                            <Star
                              className={`h-3.5 w-3.5 ${primaryIndex === idx ? 'fill-black' : ''}`}
                              aria-hidden="true"
                            />
                          </button>

                          {/* 删除单张按钮 */}
                          <button
                            type="button"
                            onClick={() => handleRemoveImage(idx)}
                            aria-label={t('modal.removePhoto')}
                            title={t('modal.removePhoto')}
                            className="absolute top-1 right-1 flex h-6 w-6 items-center justify-center rounded-md bg-black/60 text-white/70 transition-colors hover:bg-rose-500 hover:text-white focus-visible:ring-2 focus-visible:ring-rose-500/60 focus-visible:outline-none"
                          >
                            <X className="h-3.5 w-3.5" aria-hidden="true" />
                          </button>
                        </div>
                      ))}

                      {/* 添加更多按钮 */}
                      {selectedImages.length < MAX_PHOTOS && (
                        <button
                          type="button"
                          onClick={() => fileInputRef.current?.click()}
                          className={`flex aspect-square flex-col items-center justify-center rounded-xl border-2 border-dashed p-2 text-center transition-colors focus-visible:ring-2 focus-visible:ring-emerald-500/40 focus-visible:outline-none ${
                            isDragging
                              ? 'border-emerald-500 bg-emerald-500/10 text-emerald-500'
                              : 'border-[var(--border)] bg-[var(--bg-secondary)]/40 text-[var(--text-muted)] hover:border-emerald-500/50 hover:text-emerald-500'
                          }`}
                        >
                          <UploadCloud className="mb-1 h-6 w-6" aria-hidden="true" />
                          <span className="text-[10px] leading-tight font-medium">
                            {isDragging
                              ? t('modal.dropzoneActive')
                              : selectedImages.length === 0
                                ? t('modal.dropzoneText')
                                : t('modal.dropzoneMore', {
                                    remaining: MAX_PHOTOS - selectedImages.length,
                                  })}
                          </span>
                        </button>
                      )}
                    </div>

                    <input
                      ref={fileInputRef}
                      type="file"
                      accept="image/jpeg,image/png,image/webp"
                      multiple
                      className="hidden"
                      aria-label={t('modal.photoUploadTitle')}
                      onChange={handleImageSelect}
                    />
                  </div>
                )}
              </div>

              {/* ── 3. 吸底操作栏 ── */}
              <div className="flex shrink-0 items-center justify-between gap-3 border-t border-[var(--border)]/70 px-5 py-4 sm:px-6">
                <div className="hidden items-center gap-1 text-[11px] text-[var(--text-muted)] sm:flex">
                  <span>{t('modal.escHintPrefix')}</span>
                  <kbd className="rounded border border-[var(--border)] bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)] shadow-xs">
                    ESC
                  </kbd>
                  <span>{t('modal.escHintSuffix')}</span>
                </div>

                <div className="flex items-center gap-2.5">
                  <button
                    type="button"
                    onClick={handleClose}
                    disabled={isSubmitting}
                    className="inline-flex h-9 items-center justify-center rounded-xl border border-[var(--border)] px-4 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
                  >
                    {t('actions.cancel')}
                  </button>
                  <button
                    type="submit"
                    disabled={isSubmitting}
                    className="inline-flex h-9 min-w-[7rem] items-center justify-center gap-2 rounded-xl bg-emerald-500 px-5 text-xs font-semibold text-black shadow-xs transition-all hover:bg-emerald-400 focus-visible:ring-2 focus-visible:ring-emerald-500/50 focus-visible:outline-none active:scale-95 disabled:opacity-50"
                  >
                    {isSubmitting && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                    <span>{submitButtonText}</span>
                  </button>
                </div>
              </div>
            </form>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}
