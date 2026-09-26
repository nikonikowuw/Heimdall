import React, { useState, useEffect, useId } from 'react'
import {
  AlertCircle,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Clock,
  Percent,
  RefreshCw,
} from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { ModalFormHeader } from '@/components/ui/ModalFormHeader'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { motionTokens } from '@/lib/motionTokens'
import { formatTimestamp } from '@/lib/time'
import type { ReextractProgress, ReextractFaceFeaturesReport } from '@/types'

export interface ReextractModalProps {
  isOpen: boolean
  isGlobal: boolean
  targetName?: string
  initialMode?: 'confirm' | 'report'
  progress: ReextractProgress | null
  singleReport?: ReextractFaceFeaturesReport | null
  isStarting: boolean
  error?: string | null
  onClose: () => void
  onConfirm: () => void
}

function formatFailureReason(
  reason: string,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  if (!reason) return reason

  if (reason.startsWith('原始照片文件不存在:')) {
    const path = reason.replace('原始照片文件不存在:', '').trim()
    return t('reextract.reasons.photoNotFound', { path })
  }
  if (reason.startsWith('读取照片文件失败:')) {
    const error = reason.replace('读取照片文件失败:', '').trim()
    return t('reextract.reasons.readFailed', { error })
  }
  if (reason.startsWith('照片转码失败:')) {
    const error = reason.replace('照片转码失败:', '').trim()
    return t('reextract.reasons.transcodeFailed', { error })
  }
  if (
    reason.includes('未在照片中检测到有效人脸') ||
    reason.includes('未在上传照片中检测到有效人脸')
  ) {
    return t('reextract.reasons.noFaceDetected')
  }
  if (reason.startsWith('特征提取失败:')) {
    const error = reason.replace('特征提取失败:', '').trim()
    return t('reextract.reasons.extractFailed', { error })
  }
  if (reason.includes('人脸质量评分过低')) {
    const match = reason.match(/\(([0-9.]+)/)
    const score = match ? match[1] : ''
    return t('reextract.reasons.qualityTooLow', { score })
  }
  if (reason.startsWith('更新数据库特征失败:')) {
    const error = reason.replace('更新数据库特征失败:', '').trim()
    return t('reextract.reasons.dbUpdateFailed', { error })
  }

  return reason
}

export function ReextractModal({
  isOpen,
  isGlobal,
  targetName,
  initialMode = 'confirm',
  progress,
  singleReport,
  isStarting,
  error,
  onClose,
  onConfirm,
}: ReextractModalProps): React.ReactElement {
  const { t, i18n } = useTranslation(['personnel', 'common'])
  const reduceMotion = useReducedMotion()
  const [showFailures, setShowFailures] = useState(false)
  const [currentMode, setCurrentMode] = useState<'confirm' | 'progress' | 'report'>('confirm')
  const titleId = useId()
  const descriptionId = useId()
  const failuresId = useId()

  // 状态判定
  const isRunning = isGlobal ? progress?.status === 'running' || isStarting : isStarting
  const isCompleted = isGlobal
    ? progress?.status === 'completed' || progress?.status === 'failed'
    : Boolean(singleReport)

  // 根据外部状态和打开模式同步内部视图模式。
  // 同步 setState 包在微任务里，避免 effect 内同步 setState 触发级联渲染。
  useEffect(() => {
    void Promise.resolve().then(() => {
      if (!isOpen) {
        setShowFailures(false)
        return
      }
      if (isRunning) {
        setCurrentMode('progress')
      } else if (initialMode === 'report' && isCompleted) {
        setCurrentMode('report')
      } else if (isCompleted && !singleReport && initialMode !== 'confirm') {
        setCurrentMode('report')
      } else {
        setCurrentMode('confirm')
      }
    })
  }, [isOpen, isRunning, isCompleted, initialMode, singleReport])

  // 任务在当前弹窗中从运行中转为完成时，自动切换为报告展示视图
  useEffect(() => {
    if (!isCompleted) return
    void Promise.resolve().then(() => {
      setCurrentMode((current) => (current === 'progress' ? 'report' : current))
    })
  }, [isCompleted])

  // ESC 浮层栈支持（执行中可按 ESC 关闭弹窗转入后台运行）
  // Enter 仅在确认阶段触发重提（与底部主按钮一致，进行中/报告阶段不响应）
  useDismissStack(isOpen, onClose, {
    disabled: isStarting,
    onConfirm: currentMode === 'confirm' ? onConfirm : undefined,
  })

  // 汇总结算数据
  const total = isGlobal ? (progress?.total ?? 0) : (singleReport?.total ?? 0)
  const processed = isGlobal ? (progress?.processed ?? 0) : (singleReport?.total ?? 0)
  const succeeded = isGlobal ? (progress?.succeeded ?? 0) : (singleReport?.succeeded ?? 0)
  const failed = isGlobal ? (progress?.failed ?? 0) : (singleReport?.failed ?? 0)
  const failures = isGlobal ? (progress?.failures ?? []) : (singleReport?.failures ?? [])
  const percent = total > 0 ? Math.min(100, Math.round((processed / total) * 100)) : 0
  const isTaskFailed = isGlobal && progress?.status === 'failed'
  const isAllSuccess = !isTaskFailed && failed === 0

  // 任务耗时计算 (单位: 秒)
  const durationSec =
    progress?.startedAt && progress?.finishedAt && progress.finishedAt >= progress.startedAt
      ? ((progress.finishedAt - progress.startedAt) / 1000).toFixed(1)
      : null

  let completedTitle = t('reextract.successTitle')
  if (isTaskFailed) {
    completedTitle = t('reextract.failedTitle')
  }

  let completedDesc = t('reextract.partialDesc', { total, succeeded, failed })
  if (isTaskFailed) {
    completedDesc = progress?.errorMessage || t('reextract.failedDesc')
  } else if (isAllSuccess) {
    completedDesc = t('reextract.successDesc', { total })
  }

  let headerTitle = t('reextract.title')
  let headerSubtitle = isGlobal ? '' : (targetName ?? '')
  let headerBadge = 'FEATURE'
  let HeaderIcon = RefreshCw

  if (error) {
    headerTitle = t('errors.reextractFailed')
    headerSubtitle = error
    headerBadge = 'ERROR'
    HeaderIcon = AlertCircle
  } else if (currentMode === 'report') {
    headerTitle = completedTitle
    headerSubtitle = targetName ?? ''
    headerBadge = isTaskFailed ? 'FAILED' : isAllSuccess ? 'SUCCESS' : 'REPORT'
    HeaderIcon = isAllSuccess ? CheckCircle2 : AlertCircle
  } else if (isRunning || currentMode === 'progress') {
    headerTitle = t('actions.reextracting')
    headerSubtitle = t('reextract.processedRatio', { processed, total, percent })
    headerBadge = 'RUNNING'
    HeaderIcon = RefreshCw
  }

  const renderModalBody = (): React.ReactElement => {
    if (error) {
      return (
        <div
          role="alert"
          className="flex items-start gap-2.5 rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] p-3.5 text-xs text-[var(--status-danger)]"
        >
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
          <span className="leading-relaxed">{error}</span>
        </div>
      )
    }

    if (currentMode === 'report') {
      return (
        <>
          <p className="text-xs leading-relaxed text-[var(--text-secondary)]">{completedDesc}</p>

          <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
            <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 text-center">
              <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                <Check className="h-3 w-3 text-[var(--status-success)]" aria-hidden="true" />
                <span>{t('reextract.successCount')}</span>
              </div>
              <p className="font-data mt-1 text-base font-bold text-[var(--status-success)] tabular-nums">
                {succeeded}{' '}
                <span className="text-xs font-normal text-[var(--text-muted)]">/ {total}</span>
              </p>
            </div>

            <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 text-center">
              <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                <Percent className="h-3 w-3 text-[var(--status-info)]" aria-hidden="true" />
                <span>{t('reextract.successRate')}</span>
              </div>
              <p className="font-data mt-1 text-base font-bold text-[var(--status-info)] tabular-nums">
                {percent}%
              </p>
            </div>

            <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 text-center">
              <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                <AlertCircle className="h-3 w-3 text-[var(--status-warning)]" aria-hidden="true" />
                <span>{t('reextract.failedCount')}</span>
              </div>
              <p
                className={`font-data mt-1 text-base font-bold tabular-nums ${
                  failed > 0 ? 'text-[var(--status-warning)]' : 'text-[var(--text-muted)]'
                }`}
              >
                {failed}
              </p>
            </div>

            <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 text-center">
              <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                <Clock className="h-3 w-3" aria-hidden="true" />
                <span>{t('reextract.duration')}</span>
              </div>
              <p className="font-data mt-1 text-base font-bold text-[var(--text-primary)] tabular-nums">
                {durationSec ? `${durationSec}s` : '--'}
              </p>
            </div>
          </div>

          {progress?.finishedAt && (
            <p className="font-data text-right text-[10px] text-[var(--text-muted)] tabular-nums">
              {t('reextract.finishedAt')}:{' '}
              {formatTimestamp(progress.finishedAt, i18n.language || 'zh-CN')}
            </p>
          )}

          {failures.length > 0 && (
            <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3">
              <button
                type="button"
                onClick={() => setShowFailures((prev) => !prev)}
                aria-expanded={showFailures}
                aria-controls={failuresId}
                className="flex w-full items-center justify-between rounded-lg text-xs font-semibold text-[var(--text-primary)] transition-colors hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
              >
                <span className="flex items-center gap-1.5">
                  <AlertCircle
                    className="h-3.5 w-3.5 text-[var(--status-warning)]"
                    aria-hidden="true"
                  />
                  {t('reextract.failuresTitle')} ({failures.length})
                </span>
                {showFailures ? (
                  <ChevronUp className="h-4 w-4" aria-hidden="true" />
                ) : (
                  <ChevronDown className="h-4 w-4" aria-hidden="true" />
                )}
              </button>

              {showFailures && (
                <div
                  id={failuresId}
                  className="mt-2.5 max-h-48 space-y-2 overflow-y-auto pr-1 text-xs"
                >
                  {failures.map((item) => (
                    <div
                      key={item.faceId}
                      className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-2.5 text-[11px]"
                    >
                      <div className="flex flex-col gap-0.5 text-[var(--text-muted)]">
                        <span className="font-data break-all">Face: {item.faceId}</span>
                        <span className="font-data break-all">Subj: {item.subjectId}</span>
                      </div>
                      <p className="mt-1 leading-relaxed text-[var(--text-secondary)]">
                        {formatFailureReason(item.reason, t)}
                      </p>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </>
      )
    }

    if (isRunning || currentMode === 'progress') {
      return (
        <>
          <div className="flex items-end justify-between gap-3">
            <p className="text-xs leading-relaxed text-[var(--text-secondary)]">
              {t('reextract.inProgress')}
            </p>
            <span className="font-data text-xl font-bold text-[var(--accent)] tabular-nums">
              {percent}%
            </span>
          </div>

          <div
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent}
            aria-label={t('actions.reextracting')}
            className="h-2 w-full overflow-hidden rounded-full bg-[var(--bg-secondary)]"
          >
            <motion.div
              className="h-full rounded-full bg-[var(--accent)]"
              initial={false}
              animate={{ width: `${percent}%` }}
              transition={{
                duration: motionTokens.duration.fast,
                ease: motionTokens.easing.smooth,
              }}
            />
          </div>

          <div className="grid grid-cols-3 gap-2.5 text-center text-xs">
            <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5">
              <p className="text-[10px] text-[var(--text-muted)]">{t('stats.totalFaces')}</p>
              <p className="font-data mt-0.5 text-sm font-bold text-[var(--text-primary)] tabular-nums">
                {processed} / {total}
              </p>
            </div>
            <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5">
              <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                <Check className="h-3 w-3 text-[var(--status-success)]" aria-hidden="true" />
                <span>{t('reextract.successCount')}</span>
              </div>
              <p className="font-data mt-0.5 text-sm font-bold text-[var(--status-success)] tabular-nums">
                {succeeded}
              </p>
            </div>
            <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5">
              <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                <AlertCircle className="h-3 w-3 text-[var(--status-warning)]" aria-hidden="true" />
                <span>{t('reextract.failedCount')}</span>
              </div>
              <p
                className={`font-data mt-0.5 text-sm font-bold tabular-nums ${
                  failed > 0 ? 'text-[var(--status-warning)]' : 'text-[var(--text-muted)]'
                }`}
              >
                {failed}
              </p>
            </div>
          </div>

          {progress?.currentFaceId && (
            <p className="font-data truncate text-[11px] text-[var(--text-secondary)]">
              {t('reextract.currentProcessing')}: {progress.currentFaceId}
            </p>
          )}
        </>
      )
    }

    return (
      <p className="text-xs leading-relaxed text-[var(--text-secondary)]">
        {isGlobal ? t('reextract.desc') : t('reextract.singleDesc', { name: targetName || '' })}
      </p>
    )
  }

  const renderModalFooter = (): React.ReactElement => {
    if (currentMode === 'progress' && !error) {
      return (
        <div className="modal-form-footer">
          <div className="modal-form-actions">
            <button
              type="button"
              onClick={onClose}
              className="modal-form-button modal-form-button--secondary"
            >
              {t('reextract.runInBackground')}
            </button>
          </div>
        </div>
      )
    }

    return (
      <div className="modal-form-footer">
        <div className="modal-form-actions">
          {currentMode === 'report' && isGlobal && (
            <button
              type="button"
              onClick={() => setCurrentMode('confirm')}
              className="modal-form-button modal-form-button--secondary"
            >
              {t('reextract.reextractAgain')}
            </button>
          )}

          {error && (
            <>
              <button
                type="button"
                onClick={onClose}
                className="modal-form-button modal-form-button--secondary"
              >
                {t('actions.cancel')}
              </button>
              <button
                type="button"
                onClick={onConfirm}
                className="modal-form-button modal-form-button--primary"
              >
                <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
                {t('actions.retry')}
              </button>
            </>
          )}

          {!error && currentMode === 'confirm' && (
            <>
              <button
                type="button"
                onClick={onClose}
                className="modal-form-button modal-form-button--secondary"
              >
                {t('actions.cancel')}
              </button>
              <button
                type="button"
                onClick={onConfirm}
                className="modal-form-button modal-form-button--primary"
              >
                <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
                {t('actions.reextractShort')}
              </button>
            </>
          )}

          {!error && currentMode === 'report' && (
            <button
              type="button"
              onClick={onClose}
              className="modal-form-button modal-form-button--primary"
            >
              {t('actions.confirm')}
            </button>
          )}
        </div>
      </div>
    )
  }

  return (
    <AnimatePresence>
      {isOpen && (
        <div
          onClick={(e) => {
            if (e.target === e.currentTarget && !isStarting) onClose()
          }}
          className="modal-backdrop"
        >
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-labelledby={titleId}
            aria-describedby={descriptionId}
            initial={reduceMotion ? false : { opacity: 0, scale: 0.96, y: 12 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={reduceMotion ? { opacity: 0 } : { opacity: 0, scale: 0.96, y: 12 }}
            transition={{
              duration: motionTokens.duration.normal,
              ease: motionTokens.easing.smooth,
            }}
            className="modal-surface modal-surface--form max-w-xl"
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
              closeDisabled={isStarting}
            />

            <div className="modal-form-content space-y-4">{renderModalBody()}</div>

            {renderModalFooter()}
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}
