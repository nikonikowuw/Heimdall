import React, { useEffect, useRef, useState } from 'react'
import {
  AlertCircle,
  CheckCircle2,
  Circle,
  FileArchive,
  Loader2,
  ShieldCheck,
  Upload,
  X,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { algorithmApi } from '@/lib/api'
import { isAlgorithmUploadProgress } from '@/lib/uploadProgress'
import { cn } from '@/lib/utils'
import { wsClient } from '@/lib/wsClient'
import type { SandboxStepStatus, UploadAlgorithmResponse } from '@/types'
import { WS_TOPICS } from '@/types'
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

type UploadStepState = 'pending' | SandboxStepStatus

const STEP_COUNT = 6

function createUploadId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID()
  }

  const bytes = new Uint8Array(16)
  if (typeof crypto !== 'undefined' && typeof crypto.getRandomValues === 'function') {
    crypto.getRandomValues(bytes)
  } else {
    const now = Date.now()
    for (let index = 0; index < bytes.length; index += 1) {
      bytes[index] = (now >>> ((index % 4) * 8)) ^ Math.floor(Math.random() * 256)
    }
  }
  bytes[6] = (bytes[6] & 0x0f) | 0x40
  bytes[8] = (bytes[8] & 0x3f) | 0x80

  const hex = [...bytes].map((value) => value.toString(16).padStart(2, '0')).join('')
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

function createInitialStepStates(): UploadStepState[] {
  return Array.from({ length: STEP_COUNT }, () => 'pending' as const)
}

function getFinalStepStates(result: UploadAlgorithmResponse): UploadStepState[] {
  const states = createInitialStepStates()
  const passedSteps = result.passed
    ? STEP_COUNT
    : Math.min(Math.max(result.stepsPassed, 0), STEP_COUNT)

  for (let index = 0; index < passedSteps; index += 1) {
    states[index] = 'passed'
  }
  if (!result.passed && passedSteps < STEP_COUNT) {
    states[passedSteps] = 'failed'
  }
  return states
}

function stepStateClasses(state: UploadStepState): string {
  switch (state) {
    case 'running':
      return 'border-[var(--accent)] bg-[var(--accent-soft)] text-[var(--text-primary)]'
    case 'passed':
      return 'border-[var(--status-success-border)] bg-[var(--status-success-soft)] text-[var(--status-success)]'
    case 'failed':
      return 'border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)]'
    default:
      return 'border-transparent text-[var(--text-muted)]'
  }
}

function StepIcon({ state }: { state: UploadStepState }): React.ReactElement {
  if (state === 'running') {
    return <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" />
  }
  if (state === 'passed') {
    return <CheckCircle2 className="h-4 w-4" />
  }
  if (state === 'failed') {
    return <AlertCircle className="h-4 w-4" />
  }
  return <Circle className="h-4 w-4" />
}

/**
 * 算法归档包上传与沙箱自检。
 *
 * 自检进度只接受设备侧真实回流：每个步骤在对应检查边界开始时进入 running，
 * 只有该检查完成后才进入 passed。WebSocket 中断时保留未确认态，最终 HTTP 响应
 * 负责收敛结果，不用定时器模拟进度。
 */
export function UploadModal({ isOpen, onClose, onSuccess }: UploadModalProps): React.ReactElement {
  const { t } = useTranslation('algo')
  const fileInputRef = useRef<HTMLInputElement>(null)
  const progressUnsubscribeRef = useRef<(() => void) | null>(null)

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
  const [isDragging, setIsDragging] = useState(false)
  const [stepStates, setStepStates] = useState<UploadStepState[]>(createInitialStepStates)

  useDismissStack(isOpen, onClose, { disabled: isUploading })

  useEffect(() => {
    return () => {
      progressUnsubscribeRef.current?.()
      progressUnsubscribeRef.current = null
    }
  }, [])

  const stopProgressSubscription = () => {
    progressUnsubscribeRef.current?.()
    progressUnsubscribeRef.current = null
  }

  const subscribeToProgress = (uploadId: string) => {
    stopProgressSubscription()
    progressUnsubscribeRef.current = wsClient.subscribe<unknown>(
      WS_TOPICS.ALGORITHM_UPLOAD_PROGRESS,
      (payload) => {
        if (!isAlgorithmUploadProgress(payload) || payload.uploadId !== uploadId) return

        setStepStates((previous) => {
          const stepIndex = payload.step - 1
          if (previous[stepIndex] === 'passed' || previous[stepIndex] === 'failed') {
            return previous
          }
          const next = [...previous]
          next[stepIndex] = payload.status
          return next
        })
      },
    )
  }

  const applyFile = (file: File) => {
    setSelectedFile(file)
    setUploadResult(null)
    setErrorMsg(null)
    setRejection(validateAlgoPackage(file.name, file.size))
    setStepStates(createInitialStepStates())
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
    setIsDragging(false)
    const files = event.dataTransfer.files
    if (!files || files.length === 0) return
    setExtraFilesNotice(files.length > 1)
    applyFile(files[0])
  }

  const handleUploadAndVerify = async () => {
    if (!selectedFile || rejection) return

    const uploadId = createUploadId()
    subscribeToProgress(uploadId)
    setIsUploading(true)
    setErrorMsg(null)
    setUploadResult(null)
    setStepStates(createInitialStepStates())

    try {
      const res = await algorithmApi.uploadPackage(selectedFile, uploadId)
      setUploadResult(res)
      setStepStates(getFinalStepStates(res))
      if (res.passed) {
        onSuccess()
      } else {
        setErrorMsg(res.errorMessage || t('upload.failedTitle'))
      }
    } catch (error: unknown) {
      // 网络/网关错误没有可靠的失败步骤，保留已确认结果，不把错误归因给某个检查项。
      setStepStates((previous) =>
        previous.map((state) => (state === 'running' ? 'pending' : state)),
      )
      setErrorMsg(error instanceof Error ? error.message : t('upload.failedTitle'))
    } finally {
      stopProgressSubscription()
      setIsUploading(false)
    }
  }

  const handleReset = () => {
    stopProgressSubscription()
    setSelectedFile(null)
    setUploadResult(null)
    setErrorMsg(null)
    setRejection(null)
    setExtraFilesNotice(false)
    setIsDragging(false)
    setStepStates(createInitialStepStates())
  }

  const passedSteps = stepStates.filter((state) => state === 'passed').length
  const hasOutcome = Boolean(uploadResult) || errorMsg !== null
  const rejectionMessage = rejection ? t(`upload.rejection.${rejection}`) : null
  const currentStepIndex = stepStates.findIndex((state) => state === 'running')
  const firstPendingStepIndex = stepStates.findIndex((state) => state === 'pending')
  const visibleCurrentStep = currentStepIndex >= 0 ? currentStepIndex : firstPendingStepIndex

  let statusTitle = t('upload.waitingTitle')
  if (isUploading) {
    statusTitle = t('upload.testingTitle')
  } else if (uploadResult?.passed) {
    statusTitle = t('upload.passedTitle')
  } else if (errorMsg) {
    statusTitle = t('upload.failedTitle')
  }

  const getStepStatusLabel = (state: UploadStepState) => {
    if (state === 'running') return t('upload.stepStatus.running')
    if (state === 'passed') return t('upload.stepStatus.passed')
    if (state === 'failed') return t('upload.stepStatus.failed')
    return t('upload.stepStatus.pending')
  }

  return (
    <Overlay
      isOpen={isOpen}
      onClose={onClose}
      ariaLabel={t('upload.title')}
      variant="modal"
      panelClassName="max-w-xl"
    >
      <div className="flex shrink-0 items-start justify-between gap-4 border-b border-[var(--border)] pb-4">
        <div className="flex min-w-0 items-center gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl border border-[var(--accent)]/20 bg-[var(--accent-soft)] text-[var(--accent)]">
            <Upload className="h-5 w-5" />
          </div>
          <div className="min-w-0">
            <h3 className="text-sm font-bold text-[var(--text-primary)]">{t('upload.title')}</h3>
            <p className="mt-0.5 text-[11px] leading-relaxed text-[var(--text-muted)]">
              {t('upload.subtitle')}
            </p>
          </div>
        </div>
        <button
          type="button"
          onClick={onClose}
          disabled={isUploading}
          aria-label={t('actions.close')}
          title={isUploading ? t('upload.closingBlocked') : t('actions.close')}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-[var(--text-muted)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden disabled:cursor-not-allowed disabled:opacity-40"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="mt-5 min-h-0 flex-1 space-y-4 overflow-y-auto pr-1">
        {!isUploading && !uploadResult && (
          <div
            onDragOver={(event) => {
              event.preventDefault()
              setIsDragging(true)
            }}
            onDragLeave={() => setIsDragging(false)}
            onDrop={handleDrop}
            className={cn(
              'rounded-2xl border border-dashed bg-[var(--bg-secondary)]/55 p-2 transition-colors',
              isDragging
                ? 'border-[var(--accent)] bg-[var(--accent-soft)]/70'
                : 'border-[var(--border-strong)]',
            )}
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
              className="group flex min-h-32 w-full cursor-pointer items-center gap-4 rounded-xl px-5 py-6 text-left transition-colors hover:bg-[var(--accent-soft)]/45 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
            >
              <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl border border-[var(--accent)]/20 bg-[var(--accent-soft)] text-[var(--accent)] transition-transform group-hover:scale-105">
                <FileArchive className="h-6 w-6" />
              </div>
              <div className="min-w-0">
                <p className="truncate text-xs font-semibold text-[var(--text-primary)]">
                  {selectedFile ? selectedFile.name : t('upload.dragTip')}
                </p>
                <p className="mt-1 text-[11px] leading-relaxed text-[var(--text-muted)]">
                  {selectedFile
                    ? formatBytes(selectedFile.size)
                    : t('upload.fileHint', {
                        maxSize: formatBytes(500 * 1024 * 1024),
                        formats: ALGO_PACKAGE_EXTENSIONS.join(' / '),
                      })}
                </p>
              </div>
            </button>
          </div>
        )}

        {rejectionMessage && !isUploading && (
          <div
            role="alert"
            className="flex items-start gap-2.5 rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] p-3 text-xs text-[var(--status-danger)]"
          >
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
            <p className="leading-relaxed">{rejectionMessage}</p>
          </div>
        )}

        {extraFilesNotice && !isUploading && (
          <p role="status" className="text-[11px] text-[var(--text-muted)]">
            {t('upload.multipleFilesHint')}
          </p>
        )}

        {(isUploading || hasOutcome) && (
          <section
            aria-labelledby="sandbox-check-title"
            className="rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)]/45 p-4"
          >
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <h4
                  id="sandbox-check-title"
                  className="text-xs font-semibold text-[var(--text-primary)]"
                >
                  {t('upload.sandboxSectionTitle')}
                </h4>
                <p className="mt-1 text-[11px] text-[var(--text-muted)]">{statusTitle}</p>
              </div>
              <span className="font-data shrink-0 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] px-2 py-1 text-[11px] font-semibold text-[var(--text-secondary)] tabular-nums">
                {t('upload.stepCount', { passed: passedSteps, total: STEP_COUNT })}
              </span>
            </div>

            {isUploading && (
              <div className="mt-4 space-y-2">
                <div
                  role="progressbar"
                  aria-label={t('upload.testingTitle')}
                  aria-valuemin={0}
                  aria-valuemax={STEP_COUNT}
                  aria-valuenow={passedSteps}
                  className="h-1 overflow-hidden rounded-full bg-[var(--bg-secondary)]"
                >
                  <div
                    className="h-full rounded-full bg-[var(--accent)] transition-[width] duration-300 motion-reduce:transition-none"
                    style={{ width: `${(passedSteps / STEP_COUNT) * 100}%` }}
                  />
                </div>
                <p className="text-[11px] leading-relaxed text-[var(--text-muted)]">
                  {t('upload.progressHint')}
                </p>
              </div>
            )}

            {!isUploading && !uploadResult && errorMsg && (
              <p className="mt-3 text-[11px] leading-relaxed text-[var(--text-muted)]">
                {t('upload.noResultHint')}
              </p>
            )}

            <ol className="mt-4 space-y-1" aria-label={t('upload.sandboxSectionTitle')}>
              {stepsList.map((step, index) => {
                const state = stepStates[index]
                return (
                  <li
                    key={step}
                    aria-current={state === 'running' ? 'step' : undefined}
                    className={cn(
                      'flex min-h-10 items-center gap-3 rounded-xl border px-3 py-2 text-xs transition-colors',
                      stepStateClasses(state),
                    )}
                  >
                    <span className="font-data flex h-6 w-6 shrink-0 items-center justify-center rounded-lg border border-current/25 text-[11px] font-semibold tabular-nums">
                      {index + 1}
                    </span>
                    <span className="min-w-0 flex-1 leading-relaxed">{step}</span>
                    <span className="flex shrink-0 items-center gap-1.5 text-[11px] font-medium">
                      <StepIcon state={state} />
                      <span className="hidden sm:inline">{getStepStatusLabel(state)}</span>
                    </span>
                  </li>
                )
              })}
            </ol>

            {isUploading && visibleCurrentStep >= 0 && (
              <p className="mt-3 text-[11px] text-[var(--accent)]">
                {t('upload.currentStep', { step: visibleCurrentStep + 1 })}
              </p>
            )}
          </section>
        )}

        {errorMsg && (
          <div
            role="alert"
            className="flex items-start gap-2.5 rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] p-3 text-xs text-[var(--status-danger)]"
          >
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
            <p className="leading-relaxed">{errorMsg}</p>
          </div>
        )}

        {uploadResult?.passed && uploadResult.version && (
          <div className="flex items-center gap-3 rounded-xl border border-[var(--status-success-border)] bg-[var(--status-success-soft)] p-3.5 text-xs text-[var(--status-success)]">
            <ShieldCheck className="h-5 w-5 shrink-0" />
            <div className="min-w-0">
              <span className="font-bold">{t('upload.uploadSuccess')}</span>
              <p className="font-data mt-0.5 truncate text-[11px] opacity-80">
                {uploadResult.version.algorithmId} v{uploadResult.version.version} (
                {uploadResult.version.platformId})
              </p>
            </div>
          </div>
        )}
      </div>

      <div className="mt-5 flex shrink-0 flex-wrap items-center justify-end gap-2.5 border-t border-[var(--border)] pt-4">
        <button
          type="button"
          onClick={onClose}
          disabled={isUploading}
          className="inline-flex h-9 items-center justify-center rounded-xl border border-[var(--border)] px-4 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden disabled:cursor-not-allowed disabled:opacity-40"
        >
          {t('actions.close')}
        </button>

        {hasOutcome && !uploadResult?.passed && (
          <button
            type="button"
            onClick={handleReset}
            className="inline-flex h-9 items-center justify-center rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-4 text-xs font-medium text-[var(--text-primary)] transition-colors hover:border-[var(--border-strong)] hover:bg-[var(--bg-surface-solid)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
          >
            {t('upload.reupload')}
          </button>
        )}

        {!uploadResult?.passed && (
          <button
            type="button"
            disabled={!selectedFile || Boolean(rejection) || isUploading}
            onClick={handleUploadAndVerify}
            className="inline-flex h-9 items-center justify-center gap-2 rounded-xl bg-[var(--accent)] px-4 text-xs font-semibold text-white shadow-sm transition-opacity hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden disabled:cursor-not-allowed disabled:opacity-50"
          >
            {isUploading && (
              <Loader2 className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />
            )}
            <span>{t('upload.startVerify')}</span>
          </button>
        )}
      </div>
    </Overlay>
  )
}
