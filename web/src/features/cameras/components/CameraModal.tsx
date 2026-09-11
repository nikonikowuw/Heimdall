import React, { useEffect, useState } from 'react'
import { AlertCircle, Check, Loader2, Sparkles, Video, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { StreamModeSelector } from '@/components/StreamModeSelector'
import { cameraApi } from '@/lib/api'
import type { Camera, SubStreamCandidate, StreamMode } from '@/types'

export interface CameraModalProps {
  isOpen: boolean
  camera?: Camera | null
  onClose: () => void
  onSuccess: (camera: Camera) => void
}

export function CameraModal({
  isOpen,
  camera = null,
  onClose,
  onSuccess,
}: CameraModalProps): React.ReactElement | null {
  const { t } = useTranslation('camera')
  const { t: tc } = useTranslation('common')

  const isEdit = Boolean(camera)

  const [name, setName] = useState('')
  const [mainUrl, setMainUrl] = useState('')
  const [subUrl, setSubUrl] = useState('')
  const [streamMode, setStreamMode] = useState<StreamMode>('auto')
  const [remark, setRemark] = useState('')
  const [subCandidates, setSubCandidates] = useState<SubStreamCandidate[]>([])
  const [isDeducing, setIsDeducing] = useState(false)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)

  // 当弹窗打开或切换目标 camera 时，重置/初始化表单数据
  useEffect(() => {
    if (isOpen) {
      if (camera) {
        setName(camera.name || '')
        setMainUrl(camera.rtspUrl || '')
        setSubUrl(camera.subRtspUrl || '')
        setStreamMode(camera.streamMode || 'auto')
        setRemark(camera.remark || '')
      } else {
        setName('')
        setMainUrl('')
        setSubUrl('')
        setStreamMode('auto')
        setRemark('')
      }
      setSubCandidates([])
      setErrorMsg(null)
    }
  }, [isOpen, camera])

  // ESC 快捷键关闭
  useEffect(() => {
    if (!isOpen) return
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !isSubmitting) {
        onClose()
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [isOpen, isSubmitting, onClose])

  if (!isOpen) return null

  const handleDeduce = async (url: string) => {
    const trimmed = url.trim()
    if (!trimmed) return
    setIsDeducing(true)
    try {
      const candidates = await cameraApi.deduceSubStream(trimmed)
      setSubCandidates(candidates)
      if (candidates.length > 0 && !subUrl) {
        setSubUrl(candidates[0].subUrl)
      }
    } catch {
      // 容错处理：推导失败不阻断用户手动输入
    } finally {
      setIsDeducing(false)
    }
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setErrorMsg(null)

    const trimmedName = name.trim()
    const trimmedMainUrl = mainUrl.trim()
    const trimmedSubUrl = subUrl.trim()
    const trimmedRemark = remark.trim()

    if (!trimmedName) {
      setErrorMsg(t('manage.nameRequired', { defaultValue: '设备名称不能为空' }))
      return
    }
    if (!trimmedMainUrl) {
      setErrorMsg(t('manage.mainRtspRequired', { defaultValue: '主码流 RTSP 地址不能为空' }))
      return
    }

    setIsSubmitting(true)
    try {
      if (isEdit && camera) {
        const updated = await cameraApi.update(camera.cameraId, {
          name: trimmedName,
          rtspUrl: trimmedMainUrl,
          subRtspUrl: trimmedSubUrl,
          streamMode,
          remark: trimmedRemark || undefined,
        })
        onSuccess(updated)
        onClose()
      } else {
        const subRtspUrl =
          trimmedSubUrl !== '' ? trimmedSubUrl : streamMode === 'main' ? '' : undefined
        const created = await cameraApi.create({
          name: trimmedName,
          rtspUrl: trimmedMainUrl,
          subRtspUrl,
          streamMode,
          remark: trimmedRemark || undefined,
        })
        onSuccess(created)
        onClose()
      }
    } catch (err) {
      const msg =
        err instanceof Error
          ? err.message
          : t('manage.saveFailed', { defaultValue: '保存失败，请检查 RTSP 格式或网络连接' })
      setErrorMsg(msg)
    } finally {
      setIsSubmitting(false)
    }
  }

  return (
    <div
      onClick={(e) => {
        if (e.target === e.currentTarget && !isSubmitting) {
          onClose()
        }
      }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-xs"
    >
      <div className="frosted-glass relative w-full max-w-lg rounded-2xl border border-[var(--border)] p-6 shadow-2xl transition-all">
        {/* 关闭按钮 */}
        <button
          type="button"
          onClick={onClose}
          disabled={isSubmitting}
          className="absolute top-5 right-5 rounded-lg p-1 text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
        >
          <X className="h-4 w-4" />
        </button>

        {/* 头部标题与描述 */}
        <div className="flex items-start gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
            <Video className="h-5 w-5" />
          </div>
          <div>
            <h3 className="text-base font-semibold text-[var(--text-primary)]">
              {isEdit ? t('manage.editCameraTitle') : t('manage.addCameraTitle')}
            </h3>
            <p className="mt-1 text-xs text-[var(--text-muted)]">
              {isEdit ? t('manage.editCameraDesc') : t('manage.addCameraDesc')}
            </p>
          </div>
        </div>

        {/* 错误提示条 */}
        {errorMsg && (
          <div className="mt-4 flex items-center gap-2 rounded-xl border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-400">
            <AlertCircle className="h-4 w-4 shrink-0" />
            <span>{errorMsg}</span>
          </div>
        )}

        <form onSubmit={handleSubmit} className="mt-4 flex flex-col gap-4">
          {/* 设备名称 */}
          <div>
            <label className="text-xs font-medium text-[var(--text-secondary)]">
              {t('manage.name')} <span className="text-rose-500">*</span>
            </label>
            <input
              type="text"
              required
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t('manage.namePlaceholder')}
              className="mt-1 w-full rounded-xl border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-2 text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] transition-colors focus:border-[var(--accent)] focus:outline-hidden"
            />
          </div>

          {/* 主码流 RTSP 地址 */}
          <div>
            <div className="flex items-center justify-between">
              <label className="text-xs font-medium text-[var(--text-secondary)]">
                {t('manage.mainRtspUrl')} <span className="text-rose-500">*</span>
              </label>
              {isDeducing && (
                <span className="flex items-center gap-1 text-[10px] text-cyan-400">
                  <Loader2 className="h-3 w-3 animate-spin" />
                  <span>推导中...</span>
                </span>
              )}
            </div>
            <input
              type="text"
              required
              value={mainUrl}
              onChange={(e) => setMainUrl(e.target.value)}
              onBlur={() => handleDeduce(mainUrl)}
              placeholder={t('manage.mainRtspPlaceholder')}
              className="mt-1 w-full rounded-xl border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-2 font-mono text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] transition-colors focus:border-[var(--accent)] focus:outline-hidden"
            />
          </div>

          {/* 子码流 RTSP 地址 */}
          <div>
            <div className="flex items-center justify-between">
              <label className="text-xs font-medium text-[var(--text-secondary)]">
                {t('manage.subRtspUrl')}
              </label>
              {subCandidates.length > 0 && (
                <span className="flex items-center gap-1 text-[10px] font-medium text-cyan-400">
                  <Sparkles className="h-3 w-3" />
                  <span>{t('manage.subCandidatesHint')}</span>
                </span>
              )}
            </div>
            <input
              type="text"
              value={subUrl}
              onChange={(e) => setSubUrl(e.target.value)}
              placeholder={t('manage.subRtspPlaceholder')}
              className="mt-1 w-full rounded-xl border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-2 font-mono text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] transition-colors focus:border-[var(--accent)] focus:outline-hidden"
            />
            <p className="mt-1 text-[11px] text-[var(--text-muted)]">{t('manage.subRtspHint')}</p>

            {/* 子码流候选芯片预设 */}
            {subCandidates.length > 0 && (
              <div className="mt-2 flex flex-wrap gap-1.5">
                {subCandidates.map((c, i) => {
                  const isSelected = subUrl === c.subUrl
                  return (
                    <button
                      type="button"
                      key={i}
                      onClick={() => setSubUrl(c.subUrl)}
                      className={`flex items-center gap-1 rounded-lg px-2.5 py-1 font-mono text-[10px] transition-all ${
                        isSelected
                          ? 'border border-cyan-500/40 bg-cyan-500/20 font-semibold text-cyan-400'
                          : 'border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--accent)] hover:text-[var(--text-primary)]'
                      }`}
                      title={c.description}
                    >
                      {isSelected && <Check className="h-3 w-3 text-cyan-400" />}
                      <span>
                        {c.brand}: {c.description}
                      </span>
                    </button>
                  )
                })}
              </div>
            )}
          </div>

          {/* AI 分析码流偏好选择 */}
          <StreamModeSelector value={streamMode} onChange={setStreamMode} />

          {/* 备注说明 */}
          <div>
            <label className="text-xs font-medium text-[var(--text-secondary)]">
              {t('manage.remark')}
            </label>
            <input
              type="text"
              value={remark}
              onChange={(e) => setRemark(e.target.value)}
              placeholder={t('manage.remarkPlaceholder')}
              className="mt-1 w-full rounded-xl border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-2 text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] transition-colors focus:border-[var(--accent)] focus:outline-hidden"
            />
          </div>

          {/* 底部按钮栏 */}
          <div className="mt-2 flex justify-end gap-2.5">
            <button
              type="button"
              onClick={onClose}
              disabled={isSubmitting}
              className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
            >
              {t('manage.cancel', tc('actions.save'))}
            </button>
            <button
              type="submit"
              disabled={isSubmitting}
              className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95 disabled:opacity-50"
            >
              {isSubmitting && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              <span>{isSubmitting ? t('manage.saving') : t('manage.saveAndProbe')}</span>
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
