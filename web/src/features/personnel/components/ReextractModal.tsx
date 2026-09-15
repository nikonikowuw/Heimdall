import { useState, useEffect } from 'react'
import {
  RefreshCw,
  CheckCircle2,
  AlertCircle,
  ChevronDown,
  ChevronUp,
  Clock,
  Check,
  Percent,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '../../../hooks/use-dismiss-stack'
import type { ReextractProgress, ReextractFaceFeaturesReport } from '../../../types'
import { formatTimestamp } from '../../../lib/time'

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
}: ReextractModalProps) {
  const { t, i18n } = useTranslation(['personnel', 'common'])
  const [showFailures, setShowFailures] = useState(false)
  const [currentMode, setCurrentMode] = useState<'confirm' | 'progress' | 'report'>('confirm')

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

  if (!isOpen) return null

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

  let completedBadgeClass = 'border-amber-500/30 bg-amber-500/10 text-amber-400'
  if (isTaskFailed) {
    completedBadgeClass = 'border-rose-500/30 bg-rose-500/10 text-rose-400'
  } else if (isAllSuccess) {
    completedBadgeClass = 'border-emerald-500/30 bg-emerald-500/10 text-emerald-400'
  }

  let completedDesc = t('reextract.partialDesc', { total, succeeded, failed })
  if (isTaskFailed) {
    completedDesc = progress?.errorMessage || t('reextract.failedDesc')
  } else if (isAllSuccess) {
    completedDesc = t('reextract.successDesc', { total })
  }

  return (
    <div className="animate-in fade-in fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4 backdrop-blur-xs duration-200">
      <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-6 shadow-2xl">
        {/* 0. 错误状态展示 */}
        {error ? (
          <div>
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-rose-500/30 bg-rose-500/10 text-rose-400">
                <AlertCircle className="h-5 w-5" />
              </div>
              <div>
                <h3 className="text-base font-semibold text-[var(--text-primary)]">
                  {t('errors.reextractFailed')}
                </h3>
                <p className="mt-1 text-xs text-rose-400/90">{error}</p>
              </div>
            </div>

            <div className="mt-6 flex items-center justify-end gap-3">
              <button
                type="button"
                onClick={onClose}
                className="rounded-xl border border-[var(--border)] px-4 py-2 text-sm font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-tertiary)]"
              >
                {t('actions.cancel')}
              </button>
              <button
                type="button"
                onClick={onConfirm}
                className="inline-flex items-center gap-2 rounded-xl bg-emerald-500 px-4 py-2 text-sm font-semibold text-black shadow-xs transition-colors hover:bg-emerald-400"
              >
                <RefreshCw className="h-4 w-4" />
                {t('actions.retry')}
              </button>
            </div>
          </div>
        ) : currentMode === 'report' ? (
          /* 1. 完成结果详细报告展示 */
          <div>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div
                  className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border ${completedBadgeClass}`}
                >
                  {isAllSuccess ? (
                    <CheckCircle2 className="h-5 w-5" />
                  ) : (
                    <AlertCircle className="h-5 w-5" />
                  )}
                </div>
                <div>
                  <h3 className="text-base font-semibold text-[var(--text-primary)]">
                    {isTaskFailed ? t('reextract.failedTitle') : t('reextract.successTitle')}
                  </h3>
                  <p className="mt-0.5 text-xs text-[var(--text-secondary)]">{completedDesc}</p>
                </div>
              </div>
            </div>

            {/* 详细指标四宫格 */}
            <div className="mt-5 grid grid-cols-2 gap-2.5 sm:grid-cols-4">
              <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-tertiary)] p-2.5 text-center">
                <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                  <Check className="h-3 w-3 text-emerald-400" />
                  <span>{t('reextract.successCount')}</span>
                </div>
                <p className="mt-1 font-mono text-base font-bold text-emerald-400">
                  {succeeded}{' '}
                  <span className="text-xs font-normal text-[var(--text-muted)]">/ {total}</span>
                </p>
              </div>

              <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-tertiary)] p-2.5 text-center">
                <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                  <Percent className="h-3 w-3 text-cyan-400" />
                  <span>{t('reextract.successRate')}</span>
                </div>
                <p className="mt-1 font-mono text-base font-bold text-cyan-400">{percent}%</p>
              </div>

              <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-tertiary)] p-2.5 text-center">
                <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                  <AlertCircle className="h-3 w-3 text-amber-400" />
                  <span>{t('reextract.failedCount')}</span>
                </div>
                <p
                  className={`mt-1 font-mono text-base font-bold ${
                    failed > 0 ? 'text-amber-400' : 'text-[var(--text-muted)]'
                  }`}
                >
                  {failed}
                </p>
              </div>

              <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-tertiary)] p-2.5 text-center">
                <div className="flex items-center justify-center gap-1 text-[10px] text-[var(--text-muted)]">
                  <Clock className="h-3 w-3 text-[var(--text-muted)]" />
                  <span>{t('reextract.duration')}</span>
                </div>
                <p className="mt-1 font-mono text-base font-bold text-[var(--text-primary)]">
                  {durationSec ? `${durationSec}s` : '--'}
                </p>
              </div>
            </div>

            {/* 完成时间标注 */}
            {progress?.finishedAt && (
              <p className="mt-3 text-right font-mono text-[10px] text-[var(--text-muted)]">
                {t('reextract.finishedAt')}:{' '}
                {formatTimestamp(progress.finishedAt, i18n.language || 'zh-CN')}
              </p>
            )}

            {/* 失败明细折叠面板 */}
            {failures.length > 0 && (
              <div className="mt-4 rounded-xl border border-[var(--border)] bg-[var(--bg-tertiary)] p-3">
                <button
                  type="button"
                  onClick={() => setShowFailures((prev) => !prev)}
                  className="flex w-full items-center justify-between text-xs font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                >
                  <span className="flex items-center gap-1.5">
                    <AlertCircle className="h-3.5 w-3.5 text-amber-400" />
                    {t('reextract.failuresTitle')} ({failures.length})
                  </span>
                  {showFailures ? (
                    <ChevronUp className="h-4 w-4" />
                  ) : (
                    <ChevronDown className="h-4 w-4" />
                  )}
                </button>

                {showFailures && (
                  <div className="mt-2.5 max-h-48 space-y-2 overflow-y-auto pr-1 text-xs">
                    {failures.map((item) => (
                      <div
                        key={item.faceId}
                        className="rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] p-2 text-[11px]"
                      >
                        <div className="flex items-center justify-between text-[var(--text-muted)]">
                          <span className="font-mono">Face: {item.faceId.slice(0, 12)}...</span>
                          <span className="font-mono">Subj: {item.subjectId}</span>
                        </div>
                        <p className="mt-1 text-amber-400/90">
                          {formatFailureReason(item.reason, t)}
                        </p>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}

            <div className="mt-6 flex items-center justify-end gap-3">
              {isGlobal && (
                <button
                  type="button"
                  onClick={() => setCurrentMode('confirm')}
                  className="rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-tertiary)] hover:text-[var(--text-primary)]"
                >
                  {t('reextract.reextractAgain')}
                </button>
              )}
              <button
                type="button"
                onClick={onClose}
                className="rounded-xl bg-emerald-500 px-5 py-2 text-sm font-semibold text-black shadow-xs transition-colors hover:bg-emerald-400"
              >
                {t('actions.confirm')}
              </button>
            </div>
          </div>
        ) : isRunning || currentMode === 'progress' ? (
          /* 2. 实时进度展示（带进度条与统计卡片） */
          <div>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-emerald-500/30 bg-emerald-500/10 text-emerald-400">
                  <RefreshCw className="h-5 w-5 animate-spin" />
                </div>
                <div>
                  <h3 className="text-base font-semibold text-[var(--text-primary)]">
                    {t('actions.reextracting')}
                  </h3>
                  <p className="text-xs text-[var(--text-muted)]">
                    {t('reextract.processedRatio', { processed, total, percent })}
                  </p>
                </div>
              </div>
              <span className="font-mono text-xl font-bold text-emerald-400">{percent}%</span>
            </div>

            {/* 动态进度条 */}
            <div className="mt-4 h-2 w-full overflow-hidden rounded-full bg-[var(--bg-tertiary)]">
              <div
                className="h-full bg-gradient-to-r from-emerald-500 to-teal-400 transition-all duration-300 ease-out"
                style={{ width: `${percent}%` }}
              />
            </div>

            {/* 进度数据三指标卡 */}
            <div className="mt-4 grid grid-cols-3 gap-2.5 text-center text-xs">
              <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-tertiary)] p-2.5">
                <p className="text-[10px] text-[var(--text-muted)]">{t('stats.totalFaces')}</p>
                <p className="mt-0.5 font-mono text-sm font-bold text-[var(--text-primary)]">
                  {processed} / {total}
                </p>
              </div>
              <div className="rounded-xl border border-emerald-500/20 bg-emerald-500/5 p-2.5">
                <p className="text-[10px] text-emerald-400/80">{t('reextract.successCount')}</p>
                <p className="mt-0.5 font-mono text-sm font-bold text-emerald-400">{succeeded}</p>
              </div>
              <div className="rounded-xl border border-amber-500/20 bg-amber-500/5 p-2.5">
                <p className="text-[10px] text-amber-400/80">{t('reextract.failedCount')}</p>
                <p className="mt-0.5 font-mono text-sm font-bold text-amber-400">{failed}</p>
              </div>
            </div>

            {progress?.currentFaceId && (
              <p className="mt-3 truncate font-mono text-[10px] text-[var(--text-muted)]">
                {t('reextract.currentProcessing')}: {progress.currentFaceId}
              </p>
            )}

            <div className="mt-6 flex items-center justify-between border-t border-[var(--border)] pt-4">
              <span className="text-[11px] text-[var(--text-muted)]">
                {t('reextract.inProgress')}
              </span>
              <button
                type="button"
                onClick={onClose}
                className="rounded-xl border border-[var(--border)] px-4 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-tertiary)] hover:text-[var(--text-primary)]"
              >
                {t('reextract.runInBackground')}
              </button>
            </div>
          </div>
        ) : (
          /* 3. 初始二次确认阶段 */
          <div>
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-emerald-500/30 bg-emerald-500/10 text-emerald-400">
                <RefreshCw className="h-5 w-5" />
              </div>
              <h3 className="text-base font-semibold text-[var(--text-primary)]">
                {t('reextract.title')}
              </h3>
            </div>

            <p className="mt-3 text-xs leading-relaxed text-[var(--text-secondary)]">
              {isGlobal
                ? t('reextract.desc')
                : t('reextract.singleDesc', { name: targetName || '' })}
            </p>

            <div className="mt-6 flex items-center justify-end gap-3">
              <button
                type="button"
                onClick={onClose}
                className="rounded-xl border border-[var(--border)] px-4 py-2 text-sm font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-tertiary)]"
              >
                {t('actions.cancel')}
              </button>
              <button
                type="button"
                onClick={onConfirm}
                className="inline-flex items-center gap-2 rounded-xl bg-emerald-500 px-4 py-2 text-sm font-semibold text-black shadow-xs transition-colors hover:bg-emerald-400"
              >
                <RefreshCw className="h-4 w-4" />
                {t('actions.reextractShort')}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
