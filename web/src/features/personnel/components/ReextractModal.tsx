import { useState, useEffect, useId } from 'react'
import {
  AlertCircle,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Clock,
  Percent,
  RefreshCw,
  X,
} from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
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

const FOOTER_BUTTON_CLASS =
  'inline-flex h-9 items-center justify-center gap-2 rounded-xl px-4 text-xs font-medium transition-colors focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50'

const GHOST_BUTTON_CLASS = `${FOOTER_BUTTON_CLASS} border border-[var(--border)] text-[var(--text-secondary)] hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]`

const PRIMARY_BUTTON_CLASS = `${FOOTER_BUTTON_CLASS} bg-emerald-500 font-semibold text-black shadow-xs hover:bg-emerald-400 active:scale-95`

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
}: ReextractModalProps) {
  const { t, i18n } = useTranslation(['personnel', 'common'])
  const reduceMotion = useReducedMotion()
  const [showFailures, setShowFailures] = useState(false)
  const [currentMode, setCurrentMode] = useState<'confirm' | 'progress' | 'report'>('confirm')
  const titleId = useId()
  const failuresId = useId()

  // 状态判定
  const isRunning = isGlobal ? progress?.status === 'running' || isStarting : isStarting
  const isCompleted = isGlobal
    ? progress?.status === 'completed' || progress?.status === 'failed'
    : Boolean(singleReport)

  // 根据外部状态和打开模式同步内部视图模式
  useEffect(() => {
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
  }, [isOpen, isRunning, isCompleted, initialMode, singleReport])

  // 任务在当前弹窗中从运行中转为完成时，自动切换为报告展示视图
  useEffect(() => {
    if (isCompleted && currentMode === 'progress') {
      setCurrentMode('report')
    }
  }, [isCompleted, currentMode])

  // ESC 浮层栈支持（执行中可按 ESC 关闭弹窗转入后台运行）
  useDismissStack(isOpen, onClose)

  // Enter 快捷确认（仅限确认阶段）
  useEffect(() => {
    if (!isOpen || currentMode !== 'confirm') return
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Enter') {
        e.preventDefault()
        onConfirm()
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [isOpen, currentMode, onConfirm])

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

  let completedBadgeClass = 'border-amber-500/30 bg-amber-500/10 text-amber-500'
  if (isTaskFailed) {
    completedBadgeClass = 'border-rose-500/30 bg-rose-500/10 text-rose-500'
  } else if (isAllSuccess) {
    completedBadgeClass = 'border-emerald-500/30 bg-emerald-500/10 text-emerald-500'
  }

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

  let headerIconNode: React.ReactNode
  if (error) {
    headerIconNode = (
      <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-rose-500/25 bg-rose-500/10 text-rose-500 shadow-xs">
        <AlertCircle className="h-5 w-5" aria-hidden="true" />
      </div>
    )
  } else if (currentMode === 'report') {
    headerIconNode = (
      <div
        className={`flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border shadow-xs ${completedBadgeClass}`}
      >
        {isAllSuccess ? (
          <CheckCircle2 className="h-5 w-5" aria-hidden="true" />
        ) : (
          <AlertCircle className="h-5 w-5" aria-hidden="true" />
        )}
      </div>
    )
  } else {
    headerIconNode = (
      <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-emerald-500/25 bg-emerald-500/10 text-emerald-500 shadow-xs">
        <RefreshCw className={`h-5 w-5 ${isRunning ? 'animate-spin' : ''}`} aria-hidden="true" />
      </div>
    )
  }

  let headerTitle = t('reextract.title')
  let headerSubtitle: string | null = null
  if (error) {
    headerTitle = t('errors.reextractFailed')
    headerSubtitle = error
  } else if (currentMode === 'report') {
    headerTitle = completedTitle
    headerSubtitle = targetName ?? null
  } else if (isRunning || currentMode === 'progress') {
    headerTitle = t('actions.reextracting')
    headerSubtitle = t('reextract.processedRatio', { processed, total, percent })
  } else if (!isGlobal && targetName) {
    headerSubtitle = targetName
  }

  const renderModalBody = (): React.ReactElement => {
    if (error) {
      return (
        <div
          role="alert"
          className="flex items-start gap-2.5 rounded-2xl border border-rose-500/25 bg-rose-500/5 p-3.5 text-xs text-rose-500"
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
            <div className="rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/40 p-2.5 text-center">
              <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                <Check className="h-3 w-3 text-emerald-500" aria-hidden="true" />
                <span>{t('reextract.successCount')}</span>
              </div>
              <p className="font-data mt-1 text-base font-bold text-emerald-500 tabular-nums">
                {succeeded}{' '}
                <span className="text-xs font-normal text-[var(--text-muted)]">/ {total}</span>
              </p>
            </div>

            <div className="rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/40 p-2.5 text-center">
              <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                <Percent className="h-3 w-3 text-cyan-500" aria-hidden="true" />
                <span>{t('reextract.successRate')}</span>
              </div>
              <p className="font-data mt-1 text-base font-bold text-cyan-500 tabular-nums">
                {percent}%
              </p>
            </div>

            <div className="rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/40 p-2.5 text-center">
              <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                <AlertCircle className="h-3 w-3 text-amber-500" aria-hidden="true" />
                <span>{t('reextract.failedCount')}</span>
              </div>
              <p
                className={`font-data mt-1 text-base font-bold tabular-nums ${
                  failed > 0 ? 'text-amber-500' : 'text-[var(--text-muted)]'
                }`}
              >
                {failed}
              </p>
            </div>

            <div className="rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/40 p-2.5 text-center">
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
            <div className="rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/30 p-3">
              <button
                type="button"
                onClick={() => setShowFailures((prev) => !prev)}
                aria-expanded={showFailures}
                aria-controls={failuresId}
                className="flex w-full items-center justify-between rounded-lg text-xs font-medium text-[var(--text-secondary)] transition-colors hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
              >
                <span className="flex items-center gap-1.5">
                  <AlertCircle className="h-3.5 w-3.5 text-amber-500" aria-hidden="true" />
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
                      className="rounded-xl border border-[var(--border)]/70 bg-[var(--bg-surface)]/70 p-2 text-[11px]"
                    >
                      <div className="flex flex-col gap-0.5 text-[var(--text-muted)]">
                        <span className="font-data break-all">Face: {item.faceId}</span>
                        <span className="font-data break-all">Subj: {item.subjectId}</span>
                      </div>
                      <p className="mt-1 text-amber-500/90">
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
            <p className="text-xs leading-relaxed text-[var(--text-muted)]">
              {t('reextract.inProgress')}
            </p>
            <span className="font-data text-xl font-bold text-emerald-500 tabular-nums">
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
            <div
              className="h-full bg-gradient-to-r from-emerald-500 to-teal-400 transition-all duration-300 ease-out"
              style={{ width: `${percent}%` }}
            />
          </div>

          <div className="grid grid-cols-3 gap-2.5 text-center text-xs">
            <div className="rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/40 p-2.5">
              <p className="text-[10px] text-[var(--text-muted)]">{t('stats.totalFaces')}</p>
              <p className="font-data mt-0.5 text-sm font-bold text-[var(--text-primary)] tabular-nums">
                {processed} / {total}
              </p>
            </div>
            <div className="rounded-2xl border border-emerald-500/20 bg-emerald-500/5 p-2.5">
              <p className="text-[10px] text-emerald-500/80">{t('reextract.successCount')}</p>
              <p className="font-data mt-0.5 text-sm font-bold text-emerald-500 tabular-nums">
                {succeeded}
              </p>
            </div>
            <div className="rounded-2xl border border-amber-500/20 bg-amber-500/5 p-2.5">
              <p className="text-[10px] text-amber-500/80">{t('reextract.failedCount')}</p>
              <p className="font-data mt-0.5 text-sm font-bold text-amber-500 tabular-nums">
                {failed}
              </p>
            </div>
          </div>

          {progress?.currentFaceId && (
            <p className="font-data truncate text-[10px] text-[var(--text-muted)]">
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
    const escHint = (
      <div className="hidden items-center gap-1 text-[11px] text-[var(--text-muted)] sm:flex">
        <span>{t('modal.escHintPrefix')}</span>
        <kbd className="rounded border border-[var(--border)] bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)] shadow-xs">
          ESC
        </kbd>
        <span>{t('modal.escHintSuffix')}</span>
      </div>
    )

    if (currentMode === 'progress' && !error) {
      return (
        <>
          {escHint}
          <button type="button" onClick={onClose} className={GHOST_BUTTON_CLASS}>
            {t('reextract.runInBackground')}
          </button>
        </>
      )
    }

    return (
      <>
        {escHint}

        <div className="flex items-center gap-2.5">
          {currentMode === 'report' && isGlobal && (
            <button
              type="button"
              onClick={() => setCurrentMode('confirm')}
              className={GHOST_BUTTON_CLASS}
            >
              {t('reextract.reextractAgain')}
            </button>
          )}

          {error && (
            <>
              <button type="button" onClick={onClose} className={GHOST_BUTTON_CLASS}>
                {t('actions.cancel')}
              </button>
              <button type="button" onClick={onConfirm} className={PRIMARY_BUTTON_CLASS}>
                <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
                {t('actions.retry')}
              </button>
            </>
          )}

          {!error && currentMode === 'confirm' && (
            <button type="button" onClick={onConfirm} className={PRIMARY_BUTTON_CLASS}>
              <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
              {t('actions.reextractShort')}
            </button>
          )}

          {!error && currentMode === 'report' && (
            <button type="button" onClick={onClose} className={PRIMARY_BUTTON_CLASS}>
              {t('actions.confirm')}
            </button>
          )}
        </div>
      </>
    )
  }

  return (
    <AnimatePresence>
      {isOpen && (
        <div
          onClick={(e) => {
            if (e.target === e.currentTarget) onClose()
          }}
          className="fixed inset-0 z-[60] flex items-center justify-center bg-[var(--overlay-scrim)] p-4 backdrop-blur-sm"
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
            className="relative flex max-h-[92vh] w-full max-w-lg flex-col overflow-hidden rounded-[26px] border border-[var(--border)] bg-[var(--bg-surface)] shadow-[0_28px_60px_-16px_rgba(0,0,0,0.45)] backdrop-blur-2xl"
          >
            {/* 头部 */}
            <div className="flex shrink-0 items-start justify-between gap-3 border-b border-[var(--border)]/70 px-5 py-4">
              <div className="flex min-w-0 items-center gap-3.5">
                {headerIconNode}
                <div className="min-w-0">
                  <h3
                    id={titleId}
                    className="truncate text-base font-bold tracking-tight text-[var(--text-primary)]"
                  >
                    {headerTitle}
                  </h3>
                  {headerSubtitle && (
                    <p
                      className={`mt-0.5 truncate text-xs ${error ? 'text-rose-500' : 'text-[var(--text-muted)]'}`}
                    >
                      {headerSubtitle}
                    </p>
                  )}
                </div>
              </div>

              <button
                type="button"
                onClick={onClose}
                aria-label={t('common:close')}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-5">
              {renderModalBody()}
            </div>

            {/* 吸底操作栏 */}
            <div className="flex shrink-0 items-center justify-between gap-3 border-t border-[var(--border)]/70 px-5 py-4">
              {renderModalFooter()}
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}
