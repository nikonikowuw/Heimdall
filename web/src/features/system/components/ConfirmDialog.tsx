import { useEffect, useRef, useCallback } from 'react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { AlertTriangle, X } from 'lucide-react'

interface ConfirmDialogProps {
  open: boolean
  title: string
  message: string
  confirmLabel?: string
  cancelLabel?: string
  variant?: 'danger' | 'warning'
  onConfirm: () => void
  onCancel: () => void
}

export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  cancelLabel,
  variant = 'danger',
  onConfirm,
  onCancel,
}: ConfirmDialogProps): React.ReactElement {
  const { t } = useTranslation('system')
  const dialogRef = useRef<HTMLDivElement>(null)
  const confirmRef = useRef<HTMLButtonElement>(null)
  const previousActiveElement = useRef<Element | null>(null)

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onCancel()
        return
      }
      if (e.key !== 'Tab' || !dialogRef.current) return
      const focusable = dialogRef.current.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      )
      if (focusable.length === 0) return
      const first = focusable[0]
      const last = focusable[focusable.length - 1]
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault()
        last.focus()
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault()
        first.focus()
      }
    },
    [onCancel],
  )

  useEffect(() => {
    if (!open) return
    previousActiveElement.current = document.activeElement
    document.body.style.overflow = 'hidden'
    setTimeout(() => confirmRef.current?.focus(), 50)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('keydown', handleKeyDown)
      document.body.style.overflow = ''
      if (previousActiveElement.current instanceof HTMLElement) {
        previousActiveElement.current.focus()
      }
    }
  }, [open, handleKeyDown])

  const iconBg = variant === 'danger' ? 'bg-[var(--destructive)]/10' : 'bg-[var(--accent-amber)]/10'
  const iconColor =
    variant === 'danger' ? 'text-[var(--destructive)]' : 'text-[var(--accent-amber)]'
  const confirmBg =
    variant === 'danger'
      ? 'bg-[var(--destructive)] hover:bg-[var(--destructive)]/90 shadow-lg shadow-[var(--destructive)]/20'
      : 'bg-[var(--accent-amber)] hover:bg-[var(--accent-amber)]/90 shadow-lg shadow-[var(--accent-amber)]/20'

  return (
    <AnimatePresence>
      {open && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center p-4"
          role="presentation"
        >
          <motion.div
            className="fixed inset-0 bg-black/60 backdrop-blur-sm"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
            onClick={onCancel}
          />
          <motion.div
            ref={dialogRef}
            role="dialog"
            aria-modal="true"
            aria-labelledby="confirm-dialog-title"
            initial={{ opacity: 0, scale: 0.95, y: 8 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.95, y: 8 }}
            transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
            className="relative z-10 w-full max-w-md overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-surface-solid)] shadow-2xl backdrop-blur-xl"
            onClick={(e) => e.stopPropagation()}
          >
            {/* Close button */}
            <button
              onClick={onCancel}
              className="absolute top-3 right-3 rounded-lg p-1.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
            >
              <X className="h-4 w-4" />
            </button>

            <div className="p-6">
              {/* Icon */}
              <div
                className={`mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-2xl ${iconBg}`}
              >
                <AlertTriangle className={`h-7 w-7 ${iconColor}`} />
              </div>

              {/* Content */}
              <h3
                id="confirm-dialog-title"
                className="text-center text-lg font-semibold text-[var(--text-primary)]"
              >
                {title}
              </h3>
              <p className="mt-2 text-center text-[13px] leading-relaxed text-[var(--text-secondary)]">
                {message}
              </p>
            </div>

            {/* Actions */}
            <div className="flex border-t border-[var(--border)] bg-[var(--bg-secondary)]/50 px-6 py-4">
              <button
                onClick={onCancel}
                className="flex-1 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-4 py-2.5 text-sm font-medium text-[var(--text-primary)] transition-all hover:bg-[var(--bg-secondary)] active:scale-[0.98]"
              >
                {cancelLabel || t('cancel', { defaultValue: '取消' })}
              </button>
              <button
                ref={confirmRef}
                onClick={onConfirm}
                className={`ml-3 flex-1 rounded-xl px-4 py-2.5 text-sm font-medium text-white transition-all active:scale-[0.98] ${confirmBg}`}
              >
                {confirmLabel || t('confirm', { defaultValue: '确认' })}
              </button>
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}
