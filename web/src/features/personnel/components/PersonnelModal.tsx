import React, { useState, useRef, useEffect, useCallback } from 'react'
import { X, UploadCloud, Star, AlertCircle, Loader2, Image as ImageIcon } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { personnelApi } from '../../../lib/api'
import type { PersonnelItem, PersonnelDetail } from '../../../types'

interface PersonnelModalProps {
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

export const PersonnelModal: React.FC<PersonnelModalProps> = ({
  isOpen,
  onClose,
  onSuccess,
  editTarget,
  onManagePhotos,
}) => {
  const { t } = useTranslation(['personnel', 'common'])
  const isEdit = Boolean(editTarget)

  const [name, setName] = useState('')
  const [subjectId, setSubjectId] = useState('')
  const [idCard, setIdCard] = useState('')
  const [remark, setRemark] = useState('')
  const [selectedImages, setSelectedImages] = useState<ImageFilePreview[]>([])
  const [primaryIndex, setPrimaryIndex] = useState(0)

  const [isSubmitting, setIsSubmitting] = useState(false)
  const [errorMessage, setErrorMessage] = useState<string | null>(null)

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
    cleanupPreviews()
    setSelectedImages([])
    onClose()
  }

  useEffect(() => {
    if (isOpen) {
      setErrorMessage(null)
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
    } else {
      cleanupPreviews()
      setSelectedImages([])
    }
  }, [isOpen, editTarget, cleanupPreviews])

  if (!isOpen) return null

  const handleImageSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files || [])
    if (!files.length) return

    setErrorMessage(null)
    const availableSlots = 5 - selectedImages.length
    const toAdd = files.slice(0, availableSlots)

    if (files.length > availableSlots) {
      setErrorMessage(t('errors.maxPhotosExceeded'))
    }

    const newPreviews: ImageFilePreview[] = toAdd.map((file) => ({
      file,
      previewUrl: URL.createObjectURL(file),
    }))

    setSelectedImages((prev) => [...prev, ...newPreviews])
    if (fileInputRef.current) {
      fileInputRef.current.value = ''
    }
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
        cleanupPreviews()
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

        cleanupPreviews()
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

  return (
    <div className="animate-in fade-in fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4 backdrop-blur-xs duration-200">
      <div className="relative w-full max-w-xl overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] shadow-2xl">
        {/* 标题栏 */}
        <div className="flex items-center justify-between border-b border-[var(--border)] px-6 py-4">
          <h2 className="text-lg font-semibold text-[var(--text-primary)]">
            {isEdit ? t('modal.editTitle') : t('modal.registerTitle')}
          </h2>
          <button
            type="button"
            onClick={handleClose}
            className="rounded-lg p-1.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-tertiary)] hover:text-[var(--text-primary)]"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* 表单内容 */}
        <form onSubmit={handleSubmit} className="max-h-[80vh] space-y-4 overflow-y-auto p-6">
          {errorMessage && (
            <div className="flex items-start gap-2.5 rounded-xl border border-red-500/30 bg-red-500/10 p-3 text-xs text-red-400">
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <span>{errorMessage}</span>
            </div>
          )}

          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <div>
              <label className="mb-1.5 block text-xs font-medium text-[var(--text-secondary)]">
                {t('modal.name')} <span className="text-red-400">*</span>
              </label>
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t('modal.namePlaceholder')}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-tertiary)] px-3 py-2 text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:border-emerald-500/60 focus:outline-none"
                required
              />
            </div>

            <div>
              <label className="mb-1.5 block text-xs font-medium text-[var(--text-secondary)]">
                {t('modal.subjectId')}
              </label>
              <input
                type="text"
                value={subjectId}
                onChange={(e) => setSubjectId(e.target.value)}
                placeholder={t('modal.subjectIdPlaceholder')}
                disabled={isEdit}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-tertiary)] px-3 py-2 font-mono text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:border-emerald-500/60 focus:outline-none disabled:opacity-60"
              />
            </div>
          </div>

          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <div>
              <label className="mb-1.5 block text-xs font-medium text-[var(--text-secondary)]">
                {t('modal.idCard')}
              </label>
              <input
                type="text"
                value={idCard}
                onChange={(e) => setIdCard(e.target.value)}
                placeholder={t('modal.idCardPlaceholder')}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-tertiary)] px-3 py-2 text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:border-emerald-500/60 focus:outline-none"
              />
            </div>

            <div>
              <label className="mb-1.5 block text-xs font-medium text-[var(--text-secondary)]">
                {t('modal.remark')}
              </label>
              <input
                type="text"
                value={remark}
                onChange={(e) => setRemark(e.target.value)}
                placeholder={t('modal.remarkPlaceholder')}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-tertiary)] px-3 py-2 text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:border-emerald-500/60 focus:outline-none"
              />
            </div>
          </div>

          {/* 编辑模式下的人脸样本库入口提示与快捷追加 */}
          {isEdit && editTarget && (
            <div className="rounded-xl border border-emerald-500/20 bg-emerald-500/5 p-3.5">
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <ImageIcon className="h-4 w-4 text-emerald-400" />
                  <span className="text-xs font-semibold text-[var(--text-primary)]">
                    {t('modal.faceSampleManagement')} ({editTarget.faceCount}/5)
                  </span>
                </div>
                {onManagePhotos && (
                  <button
                    type="button"
                    onClick={() => {
                      handleClose()
                      onManagePhotos(editTarget)
                    }}
                    className="inline-flex items-center gap-1.5 rounded-lg border border-emerald-500/40 bg-emerald-500/10 px-2.5 py-1 text-xs font-medium text-emerald-400 transition-colors hover:bg-emerald-500/20"
                  >
                    <UploadCloud className="h-3.5 w-3.5" />
                    {t('actions.manageOrAddFaces')}
                  </button>
                )}
              </div>
              <p className="mt-1.5 text-[11px] leading-relaxed text-[var(--text-muted)]">
                {t('modal.faceSampleManagementDesc')}
              </p>
            </div>
          )}

          {/* 注册模式下的多图上传区 */}
          {!isEdit && (
            <div className="pt-2">
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                {t('modal.photoUploadTitle')} <span className="text-red-400">*</span>
              </label>
              <p className="mb-3 text-[11px] text-[var(--text-muted)]">
                {t('modal.photoUploadDesc')}
              </p>

              {/* 照片网格与上传区域 */}
              <div className="grid grid-cols-3 gap-3 sm:grid-cols-5">
                {selectedImages.map((img, idx) => (
                  <div
                    key={img.previewUrl}
                    className="group relative aspect-square overflow-hidden rounded-xl border border-[var(--border)] bg-black/40 shadow-xs"
                  >
                    <img
                      src={img.previewUrl}
                      alt={`Preview ${idx + 1}`}
                      className="h-full w-full object-cover"
                    />

                    {/* 主头像标识 / 切换按钮 */}
                    <button
                      type="button"
                      onClick={() => setPrimaryIndex(idx)}
                      className={`absolute top-1 left-1 flex h-6 w-6 items-center justify-center rounded-md text-xs transition-colors ${
                        primaryIndex === idx
                          ? 'bg-emerald-500 text-black shadow-xs'
                          : 'bg-black/60 text-white/70 hover:text-amber-400'
                      }`}
                      title={
                        primaryIndex === idx ? t('modal.primaryBadge') : t('modal.setAsPrimary')
                      }
                    >
                      <Star className={`h-3.5 w-3.5 ${primaryIndex === idx ? 'fill-black' : ''}`} />
                    </button>

                    {/* 删除单张按钮 */}
                    <button
                      type="button"
                      onClick={() => handleRemoveImage(idx)}
                      className="absolute top-1 right-1 flex h-6 w-6 items-center justify-center rounded-md bg-black/60 text-white/70 transition-colors hover:bg-red-500 hover:text-white"
                      title={t('modal.removePhoto')}
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  </div>
                ))}

                {/* 添加更多按钮 */}
                {selectedImages.length < 5 && (
                  <button
                    type="button"
                    onClick={() => fileInputRef.current?.click()}
                    className="flex aspect-square flex-col items-center justify-center rounded-xl border-2 border-dashed border-[var(--border)] bg-[var(--bg-tertiary)] p-2 text-center text-[var(--text-muted)] transition-colors hover:border-emerald-500/50 hover:text-emerald-400"
                  >
                    <UploadCloud className="mb-1 h-6 w-6" />
                    <span className="text-[10px] leading-tight font-medium">
                      {selectedImages.length === 0
                        ? t('modal.dropzoneText')
                        : `+${5 - selectedImages.length}`}
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
                onChange={handleImageSelect}
              />
            </div>
          )}

          {/* 底部按钮栏 */}
          <div className="mt-6 flex items-center justify-end gap-3 border-t border-[var(--border)] pt-4">
            <button
              type="button"
              onClick={handleClose}
              disabled={isSubmitting}
              className="rounded-xl border border-[var(--border)] px-4 py-2 text-sm font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-tertiary)]"
            >
              {t('actions.cancel')}
            </button>
            <button
              type="submit"
              disabled={isSubmitting}
              className="inline-flex items-center gap-2 rounded-xl bg-emerald-500 px-5 py-2 text-sm font-semibold text-black shadow-sm transition-all hover:bg-emerald-400 disabled:opacity-50"
            >
              {isSubmitting && <Loader2 className="h-4 w-4 animate-spin" />}
              {isSubmitting
                ? t('actions.saving')
                : isEdit
                  ? t('actions.save')
                  : t('actions.confirm')}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
