import React, { useEffect, useRef, type ReactNode } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { motionTokens } from '@/lib/motionTokens'
import { cn } from '@/lib/utils'

/** 面板形态：`modal` 居中缩放，`drawer` 从右侧推入 */
export type OverlayVariant = 'modal' | 'drawer'

export interface OverlayProps {
  isOpen: boolean
  onClose: () => void
  /** 无障碍名称（role=dialog 的 aria-label），必须是已翻译文本 */
  ariaLabel: string
  variant?: OverlayVariant
  /** 面板尺寸与布局覆盖；不传时按形态取默认值 */
  panelClassName?: string
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

const VARIANT_CLASSES: Record<OverlayVariant, { container: string; panel: string }> = {
  modal: {
    container: 'items-center justify-center p-4',
    panel: 'flex w-full max-w-2xl flex-col max-h-[85vh] rounded-3xl p-6',
  },
  drawer: {
    container: 'justify-end',
    panel: 'flex h-full w-full max-w-md flex-col border-l p-6',
  },
}

/**
 * 浮层外壳：负责退场动画、对话框语义与键盘焦点。
 *
 * 三个约定：
 * 1. **条件渲染必须在 AnimatePresence 内部**。调用方提前 `return null` 会让整棵子树
 *    在关闭瞬间被卸载，`exit` 永不触发——进有动画、退是硬切；
 * 2. `role="dialog"` + `aria-modal` + 焦点移入/归还 + Tab 循环，避免焦点落在遮罩背后的页面上；
 * 3. ESC 与 body 滚动锁仍由调用方的 `useDismissStack` 负责（各浮层的禁用条件不同）。
 */
function getPanelInitial(reducedMotion: boolean | null, isDrawer: boolean) {
  if (reducedMotion) return false
  return isDrawer ? { x: '100%' } : { opacity: 0, scale: 0.96 }
}

function getPanelExit(reducedMotion: boolean | null, isDrawer: boolean) {
  if (reducedMotion) return undefined
  return isDrawer ? { x: '100%' } : { opacity: 0, scale: 0.96 }
}

export function Overlay({
  isOpen,
  onClose,
  ariaLabel,
  variant = 'modal',
  panelClassName,
  children,
}: OverlayProps): React.ReactElement {
  const panelRef = useRef<HTMLDivElement>(null)
  const reducedMotion = useReducedMotion()
  const variantClasses = VARIANT_CLASSES[variant]

  useEffect(() => {
    if (!isOpen) return
    const panel = panelRef.current
    if (!panel) return

    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null

    // 初始焦点：优先显式标记的目标，其次第一个可聚焦元素，最后面板本身
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
        return
      }

      const first = focusable[0]
      const last = focusable[focusable.length - 1]
      const active = document.activeElement

      if (event.shiftKey && (active === first || !panel.contains(active))) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && active === last) {
        event.preventDefault()
        first.focus()
      }
    }

    panel.addEventListener('keydown', handleKeyDown)
    return () => {
      panel.removeEventListener('keydown', handleKeyDown)
      // 元素可能已被卸载，focus 在游离节点上是空操作，无需额外判空保护
      previouslyFocused?.focus({ preventScroll: true })
    }
  }, [isOpen])

  const isDrawer = variant === 'drawer'

  return (
    <AnimatePresence mode="wait">
      {isOpen && (
        <motion.div
          key="overlay"
          initial={reducedMotion ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={reducedMotion ? undefined : { opacity: 0 }}
          transition={
            reducedMotion
              ? { duration: 0 }
              : { duration: motionTokens.duration.fast, ease: motionTokens.easing.smooth }
          }
          onClick={onClose}
          className={`fixed inset-0 z-50 flex cursor-pointer bg-[var(--overlay-scrim)] backdrop-blur-xs ${variantClasses.container}`}
        >
          <motion.div
            key="overlay-panel"
            ref={panelRef}
            role="dialog"
            aria-modal="true"
            aria-label={ariaLabel}
            tabIndex={-1}
            initial={getPanelInitial(reducedMotion, isDrawer)}
            animate={isDrawer ? { x: 0 } : { opacity: 1, scale: 1 }}
            exit={getPanelExit(reducedMotion, isDrawer)}
            transition={
              reducedMotion
                ? { duration: 0 }
                : {
                    duration: motionTokens.duration.normal,
                    ease: motionTokens.easing.smooth,
                  }
            }
            onClick={(event) => event.stopPropagation()}
            className={cn(
              'frosted-glass relative z-10 cursor-default bg-[var(--bg-surface-solid)] shadow-2xl outline-hidden',
              variantClasses.panel,
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
