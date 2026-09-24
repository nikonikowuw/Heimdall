import React, { useEffect, useRef, useState } from 'react'
import { AlertCircle, AlertTriangle, CheckCircle2, ChevronRight, Info, X } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useShallow } from 'zustand/react/shallow'
import { motionTokens } from '@/lib/motionTokens'
import { toast, type ToastItem, type ToastType, useToastStore } from '@/stores/toast'

interface ToneConfig {
  icon: typeof CheckCircle2
  iconClass: string
  iconBgClass: string
  badgeClass: string
  dotClass: string
  progressClass: string
}

const TONES: Record<ToastType, ToneConfig> = {
  success: {
    icon: CheckCircle2,
    iconClass: 'text-emerald-500',
    iconBgClass: 'border-emerald-500/25 bg-emerald-500/10',
    badgeClass: 'text-emerald-600 dark:text-emerald-400 bg-emerald-500/10 border-emerald-500/20',
    dotClass: 'bg-emerald-400 shadow-[0_0_8px_rgba(16,185,129,0.7)]',
    progressClass: 'bg-emerald-500',
  },
  error: {
    icon: AlertCircle,
    iconClass: 'text-[var(--status-danger)]',
    iconBgClass: 'border-[var(--status-danger-border)] bg-[var(--status-danger-soft)]',
    badgeClass:
      'text-[var(--status-danger)] bg-[var(--status-danger-soft)] border-[var(--status-danger-border)]',
    dotClass: 'bg-[var(--status-danger)] shadow-[0_0_8px_rgba(var(--status-danger-rgb),0.7)]',
    progressClass: 'bg-[var(--status-danger)]',
  },
  warning: {
    icon: AlertTriangle,
    iconClass: 'text-amber-500',
    iconBgClass: 'border-amber-500/25 bg-amber-500/10',
    badgeClass: 'text-amber-600 dark:text-amber-400 bg-amber-500/10 border-amber-500/20',
    dotClass: 'bg-amber-400 shadow-[0_0_8px_rgba(245,158,11,0.7)]',
    progressClass: 'bg-amber-500',
  },
  info: {
    icon: Info,
    iconClass: 'text-[var(--accent)]',
    iconBgClass: 'border-[var(--accent)]/25 bg-[var(--accent-soft)]',
    badgeClass: 'text-[var(--accent)] bg-[var(--accent-soft)] border-[var(--accent)]/20',
    dotClass: 'bg-[var(--accent)] shadow-[0_0_8px_rgba(var(--accent-rgb),0.7)]',
    progressClass: 'bg-[var(--accent)]',
  },
}

export interface ToastItemViewProps {
  item: ToastItem
  onDismiss: (id: string) => void
}

export function ToastItemView({ item, onDismiss }: ToastItemViewProps): React.ReactElement {
  const { t } = useTranslation('common')
  const reducedMotion = useReducedMotion()
  const [isPaused, setIsPaused] = useState(false)
  const remainingTimeRef = useRef<number>(item.duration)
  const startTimeRef = useRef<number>(Date.now())
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const tone = TONES[item.type] || TONES.info
  const Icon = tone.icon

  // 倒计时与悬停暂停逻辑
  useEffect(() => {
    if (item.duration <= 0) return

    if (isPaused) {
      if (timerRef.current) {
        clearTimeout(timerRef.current)
        timerRef.current = null
      }
      return
    }

    startTimeRef.current = Date.now()
    timerRef.current = setTimeout(() => {
      onDismiss(item.id)
    }, remainingTimeRef.current)

    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current)
        timerRef.current = null
      }
    }
  }, [item.duration, item.id, isPaused, onDismiss])

  const handleMouseEnter = () => {
    if (item.duration <= 0) return
    const elapsed = Date.now() - startTimeRef.current
    remainingTimeRef.current = Math.max(0, remainingTimeRef.current - elapsed)
    setIsPaused(true)
  }

  const handleMouseLeave = () => {
    if (item.duration <= 0) return
    setIsPaused(false)
  }

  return (
    <motion.div
      layout
      role={item.type === 'error' ? 'alert' : 'status'}
      aria-live={item.type === 'error' ? 'assertive' : 'polite'}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      initial={reducedMotion ? { opacity: 0 } : { opacity: 0, y: -16, scale: 0.96 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={reducedMotion ? { opacity: 0 } : { opacity: 0, y: -14, scale: 0.95 }}
      transition={{
        duration: motionTokens.duration.fast,
        ease: motionTokens.easing.smooth,
      }}
      className="hud-toast-card pointer-events-auto relative w-full overflow-hidden rounded-2xl border border-[var(--border-strong)] bg-[var(--bg-surface-solid)]/95 shadow-[0_16px_36px_-10px_rgba(0,0,0,0.35),0_0_0_1px_rgba(255,255,255,0.06)] backdrop-blur-2xl transition-shadow hover:shadow-[0_20px_44px_-10px_rgba(0,0,0,0.45)]"
    >
      <div className="flex items-start gap-3.5 p-3.5 sm:p-4">
        {/* 语义图标微容器 */}
        <div
          aria-hidden="true"
          className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border transition-colors ${tone.iconBgClass} ${tone.iconClass}`}
        >
          <Icon className="h-4.5 w-4.5" />
        </div>

        {/* 内容主体区 */}
        <div className="min-w-0 flex-1 space-y-1 pt-0.5">
          {item.category && (
            <div className="flex items-center gap-2 pb-0.5">
              <span
                className={`inline-flex items-center rounded-md border px-1.5 py-0.5 font-mono text-[10px] font-semibold tracking-wider uppercase ${tone.badgeClass}`}
              >
                {item.category}
              </span>
              <span
                aria-hidden="true"
                className={`h-1.5 w-1.5 rounded-full ${tone.dotClass} animate-pulse`}
              />
            </div>
          )}

          {item.title && (
            <h4 className="text-xs font-bold tracking-tight text-[var(--text-primary)]">
              {item.title}
            </h4>
          )}

          <div className="text-xs leading-relaxed break-words text-[var(--text-secondary)] select-text">
            {item.message}
          </div>

          {/* 可交互 Action 行动按钮 */}
          {item.action && (
            <div className="pt-1.5">
              <button
                type="button"
                onClick={(e) => {
                  item.action?.onClick(e)
                  onDismiss(item.id)
                }}
                className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs font-semibold transition-all focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
                  item.action.primary
                    ? 'border-transparent bg-[var(--accent)] text-white shadow-xs hover:brightness-110'
                    : 'border-[var(--border)] bg-[var(--bg-secondary)]/80 text-[var(--text-primary)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-secondary)]'
                }`}
              >
                <span>{item.action.label}</span>
                <ChevronRight className="h-3 w-3 opacity-70" aria-hidden="true" />
              </button>
            </div>
          )}
        </div>

        {/* 手动关闭按键 */}
        <button
          type="button"
          onClick={() => onDismiss(item.id)}
          aria-label={t('close', { defaultValue: '关闭' })}
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* 底部微型进度倒计时指示条 */}
      {item.duration > 0 && !reducedMotion && (
        <div className="h-0.5 w-full overflow-hidden bg-[var(--border)]/40">
          <div
            className={`toast-progress-bar h-full origin-left ${tone.progressClass}`}
            style={{
              animation: `toast-progress ${item.duration}ms linear forwards`,
              animationPlayState: isPaused ? 'paused' : 'running',
            }}
          />
        </div>
      )}
    </motion.div>
  )
}

export interface ToasterProps {
  toasts?: ToastItem[]
}

/**
 * 全局挂载的 Toast 容器。
 *
 * 建议置于主应用视口的最顶层，采用顶部居中悬浮排列。
 */
export function Toaster({ toasts: propToasts }: ToasterProps = {}): React.ReactElement {
  const { t } = useTranslation('common')
  const storeToasts = useToastStore(useShallow((state) => state.toasts))
  const toasts = propToasts ?? storeToasts

  return (
    <div
      role="region"
      aria-label={t('notifications', { defaultValue: '系统通知' })}
      className="pointer-events-none fixed top-5 left-1/2 z-[95] flex w-[min(94vw,26rem)] -translate-x-1/2 flex-col items-center gap-2.5 sm:w-[28rem]"
    >
      <AnimatePresence mode="popLayout">
        {toasts.map((item) => (
          <ToastItemView key={item.id} item={item} onDismiss={toast.dismiss} />
        ))}
      </AnimatePresence>
    </div>
  )
}
