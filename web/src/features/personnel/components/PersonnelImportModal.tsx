import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import {
  AlertCircle,
  Archive,
  Check,
  ChevronDown,
  ChevronUp,
  Clock,
  Copy,
  Download,
  FileText,
  Info,
  ListFilter,
  Loader2,
  Percent,
  UploadCloud,
  Users,
  X,
} from 'lucide-react'
import { motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { ModalFormHeader } from '@/components/ui/ModalFormHeader'
import { ModalOverlay } from '@/components/ui/ModalOverlay'
import { personnelApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import { formatTimestamp } from '@/lib/time'
import { copyToClipboard } from '@/lib/utils'
import type { ImportFailureDetail, ImportFailureKind, PersonnelImportProgress } from '@/types'
import {
  ACCEPTED_EXTENSIONS,
  isSupportedArchiveName,
  isTerminalImportStatus,
} from './personnelImport'

export interface PersonnelImportModalProps {
  isOpen: boolean
  /** 后台任务进度快照（运行中或终态） */
  progress: PersonnelImportProgress | null
  /** 启动请求进行中 */
  isStarting: boolean
  /** 中止请求进行中 */
  isCancelling: boolean
  /** 启动/中止失败原因 */
  error?: string | null
  /** 归档单包上限（MB），与后端常量保持一致 */
  maxArchiveMb: number
  onClose: () => void
  onStart: (file: File) => void
  onCancelTask: () => void
  /** 打开报告视图（由容器决定是否携带最近一次终态报告） */
  initialMode?: 'upload' | 'progress' | 'report'
}

type ModalMode = 'upload' | 'progress' | 'report'

/** 失败归因 → i18n 标签键 */
const FAILURE_KIND_LABEL_KEY: Record<ImportFailureKind, string> = {
  transcode: 'import.transcodeTag',
  no_face: 'import.noFaceTag',
  quality_low: 'import.qualityTag',
  clash: 'import.clashTag',
  conflict: 'import.conflictTag',
  no_photo: 'import.noPhotoTag',
  internal: 'import.internalTag',
}

/** 失败归因 → 语义色 token */
const FAILURE_KIND_TONE: Record<ImportFailureKind, string> = {
  transcode:
    'border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)]',
  no_face:
    'border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] text-[var(--status-warning)]',
  quality_low:
    'border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] text-[var(--status-warning)]',
  clash:
    'border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)]',
  conflict: 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)]',
  no_photo: 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)]',
  internal:
    'border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)]',
}

/** 单条失败明细行（模块级组件：避免在 render 内重建组件类型导致整表重挂载） */
function FailureRow({
  item,
  translate,
}: {
  item: ImportFailureDetail
  translate: (key: string, options?: Record<string, unknown>) => string
}): React.ReactElement {
  return (
    <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-2.5">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-[11px] font-semibold text-[var(--text-primary)]">
          {item.name || '-'}
        </span>
        {item.subjectId ? (
          <span className="font-data text-[10px] text-[var(--text-muted)]">#{item.subjectId}</span>
        ) : (
          <span className="text-[10px] text-[var(--text-muted)]">
            {translate('import.missingPhoto')}
          </span>
        )}
        <span
          className={`rounded-md border px-1.5 py-0.5 text-[10px] font-medium ${FAILURE_KIND_TONE[item.kind]}`}
        >
          {translate(FAILURE_KIND_LABEL_KEY[item.kind])}
        </span>
        {item.skippedPhotos > 0 && (
          <span className="text-[10px] text-[var(--text-muted)]">
            {translate('import.skippedPhotos', { count: item.skippedPhotos })}
          </span>
        )}
      </div>
      <p className="mt-1 text-[11px] leading-relaxed text-[var(--text-secondary)]">{item.reason}</p>
    </div>
  )
}

/** 任务是否仍在执行（含启动请求在途） */
function isTaskActive(progress: PersonnelImportProgress | null, isStarting: boolean): boolean {
  if (isStarting) return true
  return progress?.status === 'running'
}

export function PersonnelImportModal({
  isOpen,
  progress,
  isStarting,
  isCancelling,
  error,
  maxArchiveMb,
  onClose,
  onStart,
  onCancelTask,
  initialMode = 'upload',
}: PersonnelImportModalProps): React.ReactElement {
  const { t, i18n } = useTranslation(['personnel', 'common'])
  const titleId = useId()
  const descriptionId = useId()
  const fileInputRef = useRef<HTMLInputElement>(null)

  const [mode, setMode] = useState<ModalMode>(initialMode)
  const [selectedFile, setSelectedFile] = useState<File | null>(null)
  const [isDragging, setIsDragging] = useState(false)
  const [localError, setLocalError] = useState<string | null>(null)
  const [showFailures, setShowFailures] = useState(true)
  const [kindFilter, setKindFilter] = useState<ImportFailureKind | 'all'>('all')
  const [copied, setCopied] = useState(false)

  const active = isTaskActive(progress, isStarting)
  const terminal = progress ? isTerminalImportStatus(progress.status) : false
  const failures = useMemo(() => progress?.failures ?? [], [progress])

  // 打开时按外部模式重置本地视图状态。
  // 包在微任务里：重置涉及多个 setState，同步执行会触发级联渲染。
  useEffect(() => {
    if (!isOpen) return
    void Promise.resolve().then(() => {
      setLocalError(null)
      setIsDragging(false)
      setCopied(false)
      setKindFilter('all')
      setShowFailures(failures.length > 0)
      if (initialMode === 'upload' && !active && !terminal) {
        setMode('upload')
      } else if (active) {
        setMode('progress')
      } else {
        setMode('report')
      }
    })
  }, [isOpen, initialMode, active, terminal, failures.length])

  // 任务由运行转入终态时自动切入报告视图
  useEffect(() => {
    if (!isOpen || !terminal) return
    void Promise.resolve().then(() => {
      setMode((current) => (current === 'progress' ? 'report' : current))
    })
  }, [isOpen, terminal])

  // 运行中按 ESC / 点击遮罩 = 后台运行（不中断任务）

  const pickFile = useCallback(
    (file: File | null): void => {
      setLocalError(null)
      if (!file) return
      if (!isSupportedArchiveName(file.name)) {
        setLocalError(t('import.dropzoneHint', { maxMb: maxArchiveMb }))
        return
      }
      if (file.size > maxArchiveMb * 1024 * 1024) {
        setLocalError(t('import.archiveTooLarge', { maxMb: maxArchiveMb }))
        return
      }
      setSelectedFile(file)
    },
    [t, maxArchiveMb],
  )

  const handleDrop = (event: React.DragEvent): void => {
    event.preventDefault()
    setIsDragging(false)
    const dropped = event.dataTransfer.files.item(0)
    pickFile(dropped)
  }

  const handleDownloadTemplate = async (): Promise<void> => {
    setLocalError(null)
    try {
      const blob = await personnelApi.downloadImportTemplate()
      const url = URL.createObjectURL(blob)
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = 'personnel_import_template.csv'
      document.body.appendChild(anchor)
      anchor.click()
      document.body.removeChild(anchor)
      setTimeout(() => {
        URL.revokeObjectURL(url)
      }, 1000)
    } catch (err: unknown) {
      setLocalError(err instanceof Error ? err.message : t('import.templateFailed'))
    }
  }

  const handleCopyFailures = async (): Promise<void> => {
    const text = failures
      .map((item) => [item.name || '-', item.subjectId || '-', item.reason].join('\t'))
      .join('\n')
    const ok = await copyToClipboard(text)
    setCopied(ok)
    if (!ok) setLocalError(t('import.copyFailed'))
  }

  const visibleFailures = useMemo(
    () => (kindFilter === 'all' ? failures : failures.filter((item) => item.kind === kindFilter)),
    [failures, kindFilter],
  )

  // 出现过的失败归因（用于筛选器，避免展示零命中的选项）
  const availableKinds = useMemo(() => {
    const seen = new Set<ImportFailureKind>()
    for (const item of failures) seen.add(item.kind)
    return Array.from(seen)
  }, [failures])

  const total = progress?.total ?? 0
  const processed = progress?.processed ?? 0
  const succeeded = progress?.succeeded ?? 0
  const failed = progress?.failed ?? 0
  const percent = total > 0 ? Math.min(100, Math.round((processed / total) * 100)) : 0

  const durationSec =
    progress?.startedAt && progress?.finishedAt && progress.finishedAt >= progress.startedAt
      ? ((progress.finishedAt - progress.startedAt) / 1000).toFixed(1)
      : null

  const successRate = processed > 0 ? Math.round((succeeded / processed) * 100) : 0

  // ── 报告标题/描述按终态分支 ──
  let reportTitle = t('import.reportTitleCompleted')
  let reportDesc = t('import.reportDescCompleted', { total })

  if (progress?.status === 'cancelled') {
    reportTitle = t('import.reportTitleCancelled')
    reportDesc = t('import.reportDescCancelled', { processed })
  } else if (progress?.status === 'failed') {
    reportTitle = t('import.reportTitleFailed')
    reportDesc = progress.errorMessage || t('import.reportDescFailed', { total })
  } else if (failed > 0) {
    reportTitle = t('import.reportTitlePartial')
    reportDesc = t('import.reportDescPartial', { total, succeeded, failed })
  }

  const renderUploadBody = (): React.ReactElement => (
    <>
      <div
        role="button"
        tabIndex={0}
        aria-label={t('import.dropzoneText')}
        onClick={() => fileInputRef.current?.click()}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault()
            fileInputRef.current?.click()
          }
        }}
        onDragOver={(event) => {
          event.preventDefault()
          setIsDragging(true)
        }}
        onDragLeave={() => setIsDragging(false)}
        onDrop={handleDrop}
        className={`flex cursor-pointer flex-col items-center justify-center rounded-2xl border-2 border-dashed px-5 py-8 text-center transition-colors focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
          isDragging
            ? 'border-[var(--accent)] bg-[var(--accent-soft)]'
            : 'border-[var(--border)] bg-[var(--bg-secondary)] hover:border-[var(--accent)]/50'
        }`}
      >
        <div className="mb-2.5 flex h-12 w-12 items-center justify-center rounded-2xl border border-[var(--accent)]/20 bg-[var(--accent-soft)] text-[var(--accent)]">
          <UploadCloud className="h-6 w-6" aria-hidden="true" />
        </div>
        <p className="text-xs font-semibold text-[var(--text-primary)]">
          {isDragging ? t('import.dropzoneActive') : t('import.dropzoneText')}
        </p>
        <p className="mt-1 text-[11px] text-[var(--text-secondary)]">
          {t('import.dropzoneHint', { maxMb: maxArchiveMb })}
        </p>
      </div>

      <input
        ref={fileInputRef}
        type="file"
        accept={ACCEPTED_EXTENSIONS.join(',')}
        className="hidden"
        onChange={(event) => {
          pickFile(event.target.files?.item(0) ?? null)
          event.target.value = ''
        }}
      />

      {selectedFile && (
        <div className="flex items-center gap-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3.5 py-2.5">
          <Archive className="h-4 w-4 shrink-0 text-[var(--accent)]" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <p className="text-[10px] text-[var(--text-muted)]">{t('import.selectedFile')}</p>
            <p className="truncate text-xs font-medium text-[var(--text-primary)]">
              {selectedFile.name}
            </p>
          </div>
          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            className="inline-flex h-7 shrink-0 items-center justify-center rounded-lg border border-[var(--accent)]/30 bg-[var(--accent-soft)] px-2.5 text-xs font-medium text-[var(--accent)] transition-colors hover:bg-[var(--accent)]/15 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
          >
            {t('import.changeFile')}
          </button>
        </div>
      )}

      <div className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3.5 py-2.5">
        <p className="min-w-0 flex-1 text-[11px] leading-relaxed text-[var(--text-secondary)]">
          {t('import.templateHint')}
        </p>
        <button
          type="button"
          onClick={() => void handleDownloadTemplate()}
          className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] px-3 text-xs font-medium text-[var(--text-primary)] transition-colors hover:border-[var(--accent)]/40 hover:bg-[var(--bg-secondary)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
        >
          <Download className="h-3.5 w-3.5" aria-hidden="true" />
          <span>{t('import.downloadTemplate')}</span>
        </button>
      </div>

      <div className="grid gap-2.5 sm:grid-cols-2">
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3">
          <p className="flex items-center gap-1.5 text-xs font-semibold text-[var(--text-primary)]">
            <FileText className="h-3.5 w-3.5 text-[var(--accent)]" aria-hidden="true" />
            {t('import.guideManifestTitle')}
          </p>
          <p className="mt-1.5 text-[11px] leading-relaxed text-[var(--text-secondary)]">
            {t('import.guideManifestDesc')}
          </p>
        </div>
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3">
          <p className="flex items-center gap-1.5 text-xs font-semibold text-[var(--text-primary)]">
            <Archive className="h-3.5 w-3.5 text-[var(--accent)]" aria-hidden="true" />
            {t('import.guideConventionTitle')}
          </p>
          <p className="mt-1.5 text-[11px] leading-relaxed text-[var(--text-secondary)]">
            {t('import.guideConventionDesc')}
          </p>
        </div>
      </div>

      <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3">
        <p className="flex items-center gap-1.5 text-xs font-semibold text-[var(--text-primary)]">
          <Info className="h-3.5 w-3.5 text-[var(--status-info)]" aria-hidden="true" />
          {t('import.guideTipsTitle')}
        </p>
        <ul className="mt-2 space-y-1.5">
          {[
            t('import.guideTipFace'),
            t('import.guideTipLimit'),
            t('import.guideTipClean'),
            t('import.guideTipConflict'),
            t('import.guideTipSerial'),
          ].map((tip) => (
            <li
              key={tip}
              className="flex gap-1.5 text-[11px] leading-relaxed text-[var(--text-secondary)]"
            >
              <span className="mt-1.5 h-1 w-1 shrink-0 rounded-full bg-[var(--text-muted)]" />
              <span>{tip}</span>
            </li>
          ))}
        </ul>
      </div>
    </>
  )

  const renderProgressBody = (): React.ReactElement => (
    <>
      <p className="text-xs leading-relaxed text-[var(--text-secondary)]">
        {t('import.inProgress')}
      </p>

      <div className="space-y-2">
        <div className="flex items-center justify-between text-[11px] text-[var(--text-secondary)]">
          <span>{t('import.processedRatio', { processed, total })}</span>
          <span className="font-data font-semibold text-[var(--text-primary)] tabular-nums">
            {percent}%
          </span>
        </div>
        <div
          role="progressbar"
          aria-valuenow={percent}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-label={t('import.processedRatio', { processed, total })}
          className="h-2 w-full overflow-hidden rounded-full bg-[var(--bg-secondary)]"
        >
          <motion.div
            className="h-full rounded-full bg-[var(--accent)]"
            initial={false}
            animate={{ width: `${percent}%` }}
            transition={{ duration: motionTokens.duration.fast, ease: motionTokens.easing.smooth }}
          />
        </div>
      </div>

      <div className="grid grid-cols-3 gap-2.5 text-center">
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 text-center">
          <p className="text-[10px] text-[var(--text-muted)]">{t('import.processedCount')}</p>
          <p className="font-data mt-1 text-base font-bold text-[var(--text-primary)] tabular-nums">
            {processed}
            <span className="text-xs font-normal text-[var(--text-muted)]"> / {total}</span>
          </p>
        </div>
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 text-center">
          <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
            <Check className="h-3 w-3 text-[var(--status-success)]" aria-hidden="true" />
            <span>{t('import.successCount')}</span>
          </div>
          <p className="font-data mt-1 text-base font-bold text-[var(--status-success)] tabular-nums">
            {succeeded}
          </p>
        </div>
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 text-center">
          <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
            <AlertCircle className="h-3 w-3 text-[var(--status-warning)]" aria-hidden="true" />
            <span>{t('import.failedCount')}</span>
          </div>
          <p
            className={`font-data mt-1 text-base font-bold tabular-nums ${
              failed > 0 ? 'text-[var(--status-warning)]' : 'text-[var(--text-muted)]'
            }`}
          >
            {failed}
          </p>
        </div>
      </div>

      {progress?.currentName && (
        <p className="font-data flex items-center gap-1.5 truncate text-[11px] text-[var(--text-secondary)]">
          <Loader2 className="h-3 w-3 shrink-0 animate-spin" aria-hidden="true" />
          {t('import.currentProcessing')}: {progress.currentName}
        </p>
      )}
    </>
  )

  const renderReportBody = (): React.ReactElement => (
    <>
      <p className="text-xs leading-relaxed text-[var(--text-secondary)]">{reportDesc}</p>

      <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 text-center">
          <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
            <Users className="h-3 w-3" aria-hidden="true" />
            <span>{t('pagination.totalCount')}</span>
          </div>
          <p className="font-data mt-1 text-base font-bold text-[var(--text-primary)] tabular-nums">
            {total}
          </p>
        </div>
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 text-center">
          <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
            <Check className="h-3 w-3 text-[var(--status-success)]" aria-hidden="true" />
            <span>{t('import.successCount')}</span>
          </div>
          <p className="font-data mt-1 text-base font-bold text-[var(--status-success)] tabular-nums">
            {succeeded}
          </p>
        </div>
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 text-center">
          <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
            <Percent className="h-3 w-3 text-[var(--status-info)]" aria-hidden="true" />
            <span>{t('import.successRate')}</span>
          </div>
          <p className="font-data mt-1 text-base font-bold text-[var(--status-info)] tabular-nums">
            {successRate}%
          </p>
        </div>
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 text-center">
          <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
            <Clock className="h-3 w-3" aria-hidden="true" />
            <span>{t('import.duration')}</span>
          </div>
          <p className="font-data mt-1 text-base font-bold text-[var(--text-primary)] tabular-nums">
            {durationSec ? `${durationSec}s` : '--'}
          </p>
        </div>
      </div>

      {progress?.finishedAt && (
        <p className="font-data text-right text-[10px] text-[var(--text-muted)] tabular-nums">
          {t('import.finishedAt')}: {formatTimestamp(progress.finishedAt, i18n.language || 'zh-CN')}
        </p>
      )}

      {failures.length > 0 ? (
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <button
              type="button"
              onClick={() => setShowFailures((prev) => !prev)}
              aria-expanded={showFailures}
              className="flex items-center gap-1.5 rounded-lg text-xs font-semibold text-[var(--text-primary)] transition-colors hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
            >
              <AlertCircle
                className="h-3.5 w-3.5 text-[var(--status-warning)]"
                aria-hidden="true"
              />
              {t('import.failuresTitle', { count: failures.length })}
              {showFailures ? (
                <ChevronUp className="h-4 w-4" aria-hidden="true" />
              ) : (
                <ChevronDown className="h-4 w-4" aria-hidden="true" />
              )}
            </button>

            <button
              type="button"
              onClick={() => void handleCopyFailures()}
              className="inline-flex h-7.5 items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] px-2.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)]/40 hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
            >
              {copied ? (
                <Check className="h-3.5 w-3.5 text-[var(--status-success)]" aria-hidden="true" />
              ) : (
                <Copy className="h-3.5 w-3.5" aria-hidden="true" />
              )}
              {copied ? t('import.copiedFailures') : t('import.copyFailures')}
            </button>
          </div>

          {showFailures && (
            <div className="mt-2.5 space-y-2.5">
              {availableKinds.length > 1 && (
                <div className="flex flex-wrap items-center gap-1.5">
                  <ListFilter className="h-3.5 w-3.5 text-[var(--text-muted)]" aria-hidden="true" />
                  <button
                    type="button"
                    onClick={() => setKindFilter('all')}
                    aria-pressed={kindFilter === 'all'}
                    className={`inline-flex h-6 items-center rounded-md border px-2 text-[11px] font-medium transition-colors focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
                      kindFilter === 'all'
                        ? 'border-[var(--accent)]/40 bg-[var(--accent-soft)] text-[var(--accent)]'
                        : 'border-[var(--border)] bg-[var(--bg-surface-solid)] text-[var(--text-muted)] hover:text-[var(--text-secondary)]'
                    }`}
                  >
                    {t('import.failuresFilterAll')}
                  </button>
                  {availableKinds.map((kind) => (
                    <button
                      key={kind}
                      type="button"
                      onClick={() => setKindFilter(kind)}
                      aria-pressed={kindFilter === kind}
                      className={`inline-flex h-6 items-center rounded-md border px-2 text-[11px] font-medium transition-colors focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
                        kindFilter === kind
                          ? 'border-[var(--accent)]/40 bg-[var(--accent-soft)] text-[var(--accent)]'
                          : 'border-[var(--border)] bg-[var(--bg-surface-solid)] text-[var(--text-muted)] hover:text-[var(--text-secondary)]'
                      }`}
                    >
                      {t(FAILURE_KIND_LABEL_KEY[kind])}
                    </button>
                  ))}
                </div>
              )}

              <div className="max-h-52 space-y-2 overflow-y-auto pr-1">
                {visibleFailures.map((item, index) => (
                  <FailureRow
                    key={`${item.name}-${item.subjectId}-${index}`}
                    item={item}
                    translate={t}
                  />
                ))}
              </div>
            </div>
          )}
        </div>
      ) : (
        <p className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3 py-3 text-center text-[11px] text-[var(--text-muted)]">
          {t('import.failuresEmpty')}
        </p>
      )}
    </>
  )

  const renderBody = (): React.ReactElement => {
    return (
      <div className="modal-form-content space-y-4">
        {error || localError ? (
          <div
            role="alert"
            className="flex items-start gap-2.5 rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] p-3.5 text-xs text-[var(--status-danger)]"
          >
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
            <span className="leading-relaxed">{error || localError}</span>
          </div>
        ) : null}
        {mode === 'progress' && renderProgressBody()}
        {mode === 'report' && renderReportBody()}
        {mode === 'upload' && renderUploadBody()}
      </div>
    )
  }

  let headerTitle = t('import.title')
  let headerSubtitle = t('import.subtitle')
  let headerBadge = 'IMPORT'
  let HeaderIcon = UploadCloud

  if (error || localError) {
    headerTitle = t('import.errorTitle')
    headerSubtitle = error || localError || ''
    headerBadge = 'ERROR'
    HeaderIcon = AlertCircle
  } else if (mode === 'report') {
    headerTitle = reportTitle
    headerSubtitle = t('import.reportTitleCompleted')
    headerBadge = 'REPORT'
    HeaderIcon = FileText
    if (progress?.status === 'cancelled') {
      headerSubtitle = t('import.reportTitleCancelled')
    } else if (failed > 0) {
      headerSubtitle = t('import.reportTitlePartial')
    }
  } else if (mode === 'progress') {
    headerTitle = t('import.runningShort')
    headerSubtitle = t('import.processedRatio', { processed, total })
    headerBadge = 'RUNNING'
    HeaderIcon = UploadCloud
  }

  const renderFooter = (): React.ReactElement => {
    if (mode === 'progress' && !error && !localError) {
      return (
        <div className="modal-form-footer">
          <div className="modal-form-actions">
            <button
              type="button"
              onClick={onClose}
              className="modal-form-button modal-form-button--secondary"
            >
              {t('import.runInBackground')}
            </button>
            <button
              type="button"
              onClick={onCancelTask}
              disabled={isCancelling}
              className="modal-form-button modal-form-button--danger"
            >
              {isCancelling ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <X className="h-3.5 w-3.5" aria-hidden="true" />
              )}
              {isCancelling ? t('import.cancelling') : t('import.cancelImport')}
            </button>
          </div>
        </div>
      )
    }

    if (mode === 'upload') {
      return (
        <div className="modal-form-footer">
          <div className="modal-form-actions">
            <button
              type="button"
              onClick={onClose}
              disabled={isStarting}
              className="modal-form-button modal-form-button--secondary"
            >
              {t('common:cancel')}
            </button>
            <button
              type="button"
              onClick={() => selectedFile && onStart(selectedFile)}
              disabled={!selectedFile || isStarting}
              className="modal-form-button modal-form-button--primary"
            >
              {isStarting ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <UploadCloud className="h-3.5 w-3.5" aria-hidden="true" />
              )}
              {t('import.startImport')}
            </button>
          </div>
        </div>
      )
    }

    return (
      <div className="modal-form-footer">
        <div className="modal-form-actions">
          <button
            type="button"
            onClick={() => {
              setSelectedFile(null)
              setLocalError(null)
              setMode('upload')
            }}
            className="modal-form-button modal-form-button--secondary"
          >
            {t('import.importAgain')}
          </button>
          <button
            type="button"
            onClick={onClose}
            className="modal-form-button modal-form-button--primary"
          >
            {t('import.close')}
          </button>
        </div>
      </div>
    )
  }

  return (
    <ModalOverlay
      isOpen={isOpen}
      onClose={onClose}
      ariaLabel={headerTitle}
      ariaLabelledBy={titleId}
      ariaDescribedBy={descriptionId}
      surface="solid"
      closeDisabled={isStarting || isCancelling}
      panelClassName="modal-surface--form max-w-xl p-0"
    >
      <ModalFormHeader
        icon={HeaderIcon}
        title={headerTitle}
        titleId={titleId}
        description={headerSubtitle}
        descriptionId={descriptionId}
        badge={headerBadge}
        closeLabel={t('common:close')}
        onClose={onClose}
        closeDisabled={isStarting || isCancelling}
      />

      {renderBody()}

      {renderFooter()}
    </ModalOverlay>
  )
}
