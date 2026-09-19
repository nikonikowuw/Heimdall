import React from 'react'
import { AlertCircle, CheckCircle2, ChevronRight, FileText, X } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '../../../lib/motionTokens'

export type PersonnelNoticeType = 'success' | 'warning' | 'error'

export interface PersonnelNotice {
  id: number
  title: string
  message: string
  type: PersonnelNoticeType
  /** 仅在特征重提任务收尾时出现「查看提取报告」行动项 */
  action?: 'report'
}

interface NoticeTone {
  icon: typeof CheckCircle2
  iconClass: string
  iconBgClass: string
  accentClass: string
}

const TONES: Record<PersonnelNoticeType, NoticeTone> = {
  success: {
    icon: CheckCircle2,
    iconClass: 'text-emerald-500',
    iconBgClass: 'border-emerald-500/25 bg-emerald-500/10',
    accentClass: 'bg-emerald-500',
  },
  warning: {
    icon: AlertCircle,
    iconClass: 'text-amber-500',
    iconBgClass: 'border-amber-500/25 bg-amber-500/10',
    accentClass: 'bg-amber-500',
  },
  error: {
    icon: AlertCircle,
    iconClass: 'text-rose-500',
    iconBgClass: 'border-rose-500/25 bg-rose-500/10',
    accentClass: 'bg-rose-500',
  },
}

export interface PersonnelToastProps {
  notice: PersonnelNotice | null
  onDismiss: () => void
  onOpenReport: () => void
}

/**
 * 右上角即时反馈卡片。
 *
 * 完全受控：超时收起由持有 `notice` 的页面容器负责，卡片自身只做呈现，
 * 避免内部计时器与列表刷新竞态后出现「已消失又复现」的重复提示。
 */
export function PersonnelToast({
  notice,
  onDismiss,
  onOpenReport,
}: PersonnelToastProps): React.ReactElement {
  const { t } = useTranslation(['personnel', 'common'])
  const reduceMotion = useReducedMotion()

  return (
    <AnimatePresence mode="wait">
      {notice && (
        <motion.div
          key={notice.id}
          role="status"
          aria-live="polite"
          initial={reduceMotion ? false : { opacity: 0, y: -16, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={reduceMotion ? { opacity: 0 } : { opacity: 0, y: -12, scale: 0.97 }}
          transition={{ duration: motionTokens.duration.fast, ease: motionTokens.easing.smooth }}
          className="fixed top-4 right-4 z-[80] w-[min(92vw,26rem)] overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-surface)] shadow-[0_20px_44px_-14px_rgba(0,0,0,0.35)] backdrop-blur-2xl sm:top-5 sm:right-5"
        >
          <div className="flex items-start gap-3 p-4">
            <span
              aria-hidden="true"
              className={`mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-xl border ${TONES[notice.type].iconBgClass} ${TONES[notice.type].iconClass}`}
            >
              {React.createElement(TONES[notice.type].icon, { className: 'h-4 w-4' })}
            </span>

            <div className="min-w-0 flex-1">
              <h4 className="text-xs font-semibold tracking-tight text-[var(--text-primary)]">
                {notice.title}
              </h4>
              <p className="mt-0.5 text-xs leading-relaxed break-words text-[var(--text-secondary)]">
                {notice.message}
              </p>

              {notice.action === 'report' && (
                <button
                  type="button"
                  onClick={() => {
                    onDismiss()
                    onOpenReport()
                  }}
                  className="mt-2 inline-flex items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)]/60 px-2 py-1 text-[11px] font-medium text-[var(--text-secondary)] transition-colors hover:border-emerald-500/40 hover:text-emerald-500 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                >
                  <FileText className="h-3 w-3" aria-hidden="true" />
                  {t('reextract.viewReport')}
                  <ChevronRight className="h-3 w-3 opacity-60" aria-hidden="true" />
                </button>
              )}
            </div>

            <button
              type="button"
              onClick={onDismiss}
              aria-label={t('common:close')}
              className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>

          {/* 底部色彩指示条：类型识别不依赖图标形状 */}
          <div aria-hidden="true" className={`h-0.5 w-full ${TONES[notice.type].accentClass}`} />
        </motion.div>
      )}
    </AnimatePresence>
  )
}
