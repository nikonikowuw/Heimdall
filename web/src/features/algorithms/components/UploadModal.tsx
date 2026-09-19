import React, { useRef, useState } from 'react'
import {
  AlertCircle,
  CheckCircle2,
  FileArchive,
  Loader2,
  ShieldCheck,
  Upload,
  X,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { algorithmApi } from '@/lib/api'
import type { UploadAlgorithmResponse } from '@/types'
import { formatBytes } from '../format'
import {
  ALGO_PACKAGE_EXTENSIONS,
  validateAlgoPackage,
  type AlgoPackageRejection,
} from '../uploadValidation'
import { Overlay } from './Overlay'

export interface UploadModalProps {
  isOpen: boolean
  onClose: () => void
  onSuccess: () => void
}

/**
 * 算法归档包上传与沙箱自检。
 *
 * 进度表达必须诚实：服务端只在自检结束后返回逐项结果，中途没有进度回流，
 * 因此进行中只呈现不确定态进度条 + 「进行中」文案，不用定时器演一个假的逐格推进
 * （那会导致失败时步进回退、小包上传时整列跳绿这类自相矛盾的画面）。
 */
export function UploadModal({ isOpen, onClose, onSuccess }: UploadModalProps): React.ReactElement {
  const { t } = useTranslation('algo')
  const fileInputRef = useRef<HTMLInputElement>(null)

  const stepsList = [
    t('upload.step1'),
    t('upload.step2'),
    t('upload.step3'),
    t('upload.step4'),
    t('upload.step5'),
    t('upload.step6'),
  ]

  const [selectedFile, setSelectedFile] = useState<File | null>(null)
  const [rejection, setRejection] = useState<AlgoPackageRejection | null>(null)
  const [isUploading, setIsUploading] = useState(false)
  const [uploadResult, setUploadResult] = useState<UploadAlgorithmResponse | null>(null)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)
  const [extraFilesNotice, setExtraFilesNotice] = useState(false)

  useDismissStack(isOpen, onClose, { disabled: isUploading })

  const applyFile = (file: File) => {
    setSelectedFile(file)
    setUploadResult(null)
    setErrorMsg(null)
    setRejection(validateAlgoPackage(file.name, file.size))
  }

  const handleFileSelect = (event: React.ChangeEvent<HTMLInputElement>) => {
    setExtraFilesNotice(false)
    const file = event.target.files?.[0]
    if (file) applyFile(file)
    // 允许重复选中同一个文件后再次触发变更事件
    event.target.value = ''
  }

  const handleDrop = (event: React.DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    const files = event.dataTransfer.files
    if (!files || files.length === 0) return
    setExtraFilesNotice(files.length > 1)
    applyFile(files[0])
  }

  const handleUploadAndVerify = async () => {
    if (!selectedFile || rejection) return

    setIsUploading(true)
    setErrorMsg(null)
    setUploadResult(null)

    try {
      const res = await algorithmApi.uploadPackage(selectedFile)
      setUploadResult(res)
      if (res.passed) {
        onSuccess()
      } else {
        setErrorMsg(res.errorMessage || t('upload.failedTitle'))
      }
    } catch (error: unknown) {
      setErrorMsg(error instanceof Error ? error.message : t('upload.failedTitle'))
    } finally {
      setIsUploading(false)
    }
  }

  const handleReset = () => {
    setSelectedFile(null)
    setUploadResult(null)
    setErrorMsg(null)
    setRejection(null)
    setExtraFilesNotice(false)
  }

  const passedSteps = uploadResult ? Math.min(uploadResult.stepsPassed, stepsList.length) : 0
  const hasOutcome = Boolean(uploadResult) || errorMsg !== null
  const rejectionMessage = rejection ? t(`upload.rejection.${rejection}`) : null

  const renderStepState = (index: number) => {
    if (!hasOutcome) {
      return { className: 'text-[var(--text-muted)] opacity-60', icon: null }
    }
    if (index < passedSteps) {
      return {
        className: 'bg-emerald-500/10 text-emerald-400',
        icon: <CheckCircle2 className="h-4 w-4 text-emerald-400" />,
      }
    }
    if (errorMsg && index === passedSteps) {
      return {
        className: 'bg-red-500/10 text-red-400',
        icon: <AlertCircle className="h-4 w-4 text-red-400" />,
      }
    }
    return { className: 'text-[var(--text-muted)] opacity-50', icon: null }
  }

  let statusTitle = t('upload.testingTitle')
  if (uploadResult?.passed) {
    statusTitle = t('upload.passedTitle')
  } else if (errorMsg) {
    statusTitle = t('upload.failedTitle')
  }

  return (
    <Overlay
      isOpen={isOpen}
      onClose={onClose}
      ariaLabel={t('upload.title')}
      variant="modal"
      panelClassName="max-w-lg"
    >
      <div className="flex items-center justify-between border-b border-[var(--border)] pb-4">
        <div className="flex items-center gap-2.5">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
            <Upload className="h-5 w-5" />
          </div>
          <div>
            <h3 className="text-sm font-bold text-[var(--text-primary)]">{t('upload.title')}</h3>
            <p className="text-[11px] text-[var(--text-muted)]">{t('upload.subtitle')}</p>
          </div>
        </div>
        <button
          type="button"
          onClick={onClose}
          disabled={isUploading}
          aria-label={t('actions.close')}
          title={isUploading ? t('upload.closingBlocked') : t('actions.close')}
          className="flex h-7 w-7 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden disabled:opacity-40"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="mt-4 space-y-4">
        {!isUploading && !uploadResult && (
          <div
            onDragOver={(event) => event.preventDefault()}
            onDrop={handleDrop}
            className="rounded-2xl border-2 border-dashed border-[var(--border)] bg-[var(--bg-secondary)] p-2 transition-colors focus-within:border-[var(--accent)]"
          >
            <input
              ref={fileInputRef}
              type="file"
              accept={ALGO_PACKAGE_EXTENSIONS.join(',')}
              onChange={handleFileSelect}
              className="hidden"
            />
            <button
              type="button"
              data-autofocus
              onClick={() => fileInputRef.current?.click()}
              className="flex w-full cursor-pointer flex-col items-center justify-center rounded-xl px-6 py-8 text-center transition-colors hover:bg-[var(--accent-soft)]/10 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
            >
              <FileArchive className="h-10 w-10 text-[var(--accent)]" />
              <span className="mt-3 text-xs font-semibold text-[var(--text-primary)]">
                {selectedFile ? selectedFile.name : t('upload.dragTip')}
              </span>
              <span className="mt-1 text-[11px] text-[var(--text-muted)]">
                {selectedFile
                  ? formatBytes(selectedFile.size)
                  : t('upload.fileHint', {
                      maxSize: formatBytes(500 * 1024 * 1024),
                      formats: ALGO_PACKAGE_EXTENSIONS.join(' / '),
                    })}
              </span>
            </button>
          </div>
        )}

        {rejectionMessage && !isUploading && (
          <p role="alert" className="text-[11px] text-red-400">
            {rejectionMessage}
          </p>
        )}

        {extraFilesNotice && !isUploading && (
          <p role="status" className="text-[11px] text-[var(--text-muted)]">
            {t('upload.multipleFilesHint')}
          </p>
        )}

        {(isUploading || hasOutcome) && (
          <div className="rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4">
            <div className="mb-3 flex items-center justify-between gap-3 text-xs font-semibold">
              <span className="text-[var(--text-primary)]">{statusTitle}</span>
              {hasOutcome && (
                <span className="font-data text-[var(--accent)] tabular-nums">
                  {passedSteps} / {stepsList.length}
                </span>
              )}
            </div>

            {isUploading && (
              <div className="mb-3 space-y-1.5">
                <div
                  role="progressbar"
                  aria-label={t('upload.testingTitle')}
                  className="indeterminate-track h-1 w-full"
                >
                  <div className="indeterminate-bar" />
                </div>
                <p className="text-[11px] text-[var(--text-muted)]">{t('upload.progressHint')}</p>
              </div>
            )}

            <div className="space-y-2" role="status" aria-live="polite">
              {stepsList.map((step, idx) => {
                const state = renderStepState(idx)
                return (
                  <div
                    key={step}
                    className={`flex items-center justify-between rounded-xl px-3 py-2 text-xs transition-colors ${
                      isUploading ? 'text-[var(--text-muted)] opacity-60' : state.className
                    }`}
                  >
                    <span className="font-medium">{step}</span>
                    {isUploading && idx === 0 ? (
                      <Loader2 className="h-4 w-4 animate-spin text-[var(--accent)]" />
                    ) : (
                      state.icon
                    )}
                  </div>
                )
              })}
            </div>
          </div>
        )}

        {errorMsg && (
          <div
            role="alert"
            className="flex items-start gap-2.5 rounded-xl border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-400"
          >
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
            <p className="leading-relaxed">{errorMsg}</p>
          </div>
        )}

        {uploadResult?.passed && uploadResult.version && (
          <div className="flex items-center gap-3 rounded-xl border border-emerald-500/20 bg-emerald-500/10 p-3.5 text-xs text-emerald-400">
            <ShieldCheck className="h-5 w-5 shrink-0" />
            <div>
              <span className="font-bold">{t('upload.uploadSuccess')}</span>
              <p className="font-data mt-0.5 text-[11px] opacity-80">
                {uploadResult.version.algorithmId} v{uploadResult.version.version} (
                {uploadResult.version.platformId})
              </p>
            </div>
          </div>
        )}
      </div>

      <div className="mt-6 flex items-center justify-end gap-2.5 border-t border-[var(--border)] pt-4">
        <button
          type="button"
          onClick={onClose}
          disabled={isUploading}
          className="rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] disabled:opacity-40"
        >
          {t('actions.close')}
        </button>

        {hasOutcome && !uploadResult?.passed && (
          <button
            type="button"
            onClick={handleReset}
            className="rounded-xl bg-[var(--bg-secondary)] px-4 py-2 text-xs font-medium text-[var(--text-primary)] transition-colors hover:bg-[var(--border)]"
          >
            {t('upload.reupload')}
          </button>
        )}

        {!uploadResult?.passed && (
          <button
            type="button"
            disabled={!selectedFile || Boolean(rejection) || isUploading}
            onClick={handleUploadAndVerify}
            className="flex items-center gap-2 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-md transition-all hover:opacity-90 disabled:opacity-50"
          >
            {isUploading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
            <span>{t('upload.startVerify')}</span>
          </button>
        )}
      </div>
    </Overlay>
  )
}
