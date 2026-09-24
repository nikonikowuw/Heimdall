import { useEffect, useRef, type ReactNode, type ReactElement } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { motionTokens } from '@/lib/motionTokens'
import { cn } from '@/lib/utils'

export type ModalOverlayVariant = 'modal' | 'drawer'
export type ModalOverlaySurface = 'glass' | 'solid'

export interface ModalOverlayProps {
  isOpen: boolean
  onClose: () => void
  ariaLabel: string
  ariaLabelledBy?: string
  ariaDescribedBy?: string
  role?: 'dialog' | 'alertdialog'
  variant?: ModalOverlayVariant
  surface?: ModalOverlaySurface
  panelClassName?: string
  closeDisabled?: boolean
  priority?: number
  children: ReactNode
}

const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ')

function getPanelHiddenState(
  isDrawer: boolean,
): { x: string } | { opacity: number; scale: number } {
  return isDrawer ? { x: '100%' } : { opacity: 0, scale: 0.96 }
}

export function ModalOverlay({
  isOpen,
  onClose,
  ariaLabel,
  ariaLabelledBy,
  ariaDescribedBy,
  role = 'dialog',
  variant = 'modal',
  surface = 'glass',
  panelClassName,
  closeDisabled = false,
  priority = 0,
  children,
}: ModalOverlayProps): ReactElement {
  const panelRef = useRef<HTMLDivElement>(null)
  const reducedMotion = useReducedMotion()
  const isDrawer = variant === 'drawer'

  useDismissStack(isOpen, onClose, { disabled: closeDisabled, priority })

  useEffect(() => {
    if (!isOpen) return
    const panel = panelRef.current
    if (!panel) return

    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null
    const initialTarget =
      panel.querySelector<HTMLElement>('[data-autofocus]') ??
      panel.querySelector<HTMLElement>(FOCUSABLE_SELECTOR) ??
      panel
    initialTarget.focus({ preventScroll: true })

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Tab') return

      const focusable = [...panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)].filter(
        (element) => element.offsetParent !== null || element === document.activeElement,
      )
      if (focusable.length === 0) {
        event.preventDefault()
        panel.focus()
        return
      }

      const first = focusable[0]
      const last = focusable[focusable.length - 1]
      const active = document.activeElement

      if (event.shiftKey && (active === first || !panel.contains(active))) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && (active === last || !panel.contains(active))) {
        event.preventDefault()
        first.focus()
      }
    }

    panel.addEventListener('keydown', handleKeyDown)
    return () => {
      panel.removeEventListener('keydown', handleKeyDown)
      previouslyFocused?.focus({ preventScroll: true })
    }
  }, [isOpen])

  return (
    <AnimatePresence mode="wait">
      {isOpen && (
        <motion.div
          key="modal-backdrop"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={reducedMotion ? undefined : { opacity: 0 }}
          transition={
            reducedMotion
              ? { duration: 0 }
              : { duration: motionTokens.duration.fast, ease: motionTokens.easing.smooth }
          }
          onClick={(event) => {
            if (!closeDisabled && event.target === event.currentTarget) {
              onClose()
            }
          }}
          className={cn('modal-backdrop cursor-pointer', isDrawer && 'modal-backdrop--drawer')}
        >
          <motion.div
            key="modal-panel"
            ref={panelRef}
            role={role}
            aria-modal="true"
            aria-label={ariaLabelledBy ? undefined : ariaLabel}
            aria-labelledby={ariaLabelledBy}
            aria-describedby={ariaDescribedBy}
            tabIndex={-1}
            initial={reducedMotion ? false : getPanelHiddenState(isDrawer)}
            animate={isDrawer ? { x: 0 } : { opacity: 1, scale: 1 }}
            exit={reducedMotion ? undefined : getPanelHiddenState(isDrawer)}
            transition={
              reducedMotion
                ? { duration: 0 }
                : { duration: motionTokens.duration.normal, ease: motionTokens.easing.smooth }
            }
            onClick={(event) => event.stopPropagation()}
            className={cn(
              // p-6 为默认内衬；共享表单布局由调用方覆盖内边距，并使用自己的内容区和页脚间距
              'modal-surface cursor-default p-6 outline-hidden',
              surface === 'glass' && 'modal-surface--glass',
              isDrawer ? 'modal-surface--drawer' : 'modal-surface--wide',
              panelClassName,
            )}
          >
            {children}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}
