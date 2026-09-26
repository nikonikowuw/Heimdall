import { useRef, type ReactNode, type ReactElement } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { useFocusTrap } from '@/hooks/use-focus-trap'
import { motionTokens } from '@/lib/motionTokens'
import { cn } from '@/lib/utils'

export type ModalOverlayVariant = 'modal' | 'drawer'
export type ModalOverlaySurface = 'glass' | 'solid'
export type ModalOverlayLayer = 'base' | 'raised' | 'top' | 'highest'

/** 层级 token 映射：禁止调用点在组件内写 `z-[70]` 这类魔法值。 */
const LAYER_CLASS: Record<ModalOverlayLayer, string> = {
  base: '',
  raised: 'modal-backdrop--raised',
  top: 'modal-backdrop--top',
  highest: 'modal-backdrop--highest',
}

export interface ModalOverlayProps {
  isOpen: boolean
  onClose: () => void
  ariaLabel: string
  ariaLabelledBy?: string
  ariaDescribedBy?: string
  role?: 'dialog' | 'alertdialog'
  variant?: ModalOverlayVariant
  surface?: ModalOverlaySurface
  /** 浮层层级，默认为基础 modal 层 */
  layer?: ModalOverlayLayer
  panelClassName?: string
  backdropClassName?: string
  closeDisabled?: boolean
  /** 浮层栈优先级，数值越大越优先被 ESC 关闭（嵌套浮层用） */
  priority?: number
  /** 传入后接管 Enter 确认（确认类弹窗的键盘契约） */
  onConfirm?: () => void
  /** 退出动画结束回调；调用方据此清理为动画保留的最后一份数据快照 */
  onExitComplete?: () => void
  children: ReactNode
}

function getPanelHiddenState(
  isDrawer: boolean,
): { x: string } | { opacity: number; scale: number } {
  return isDrawer ? { x: '100%' } : { opacity: 0, scale: 0.96 }
}

/**
 * 统一浮层外壳：承担遮罩、层级、进出场动效、焦点陷阱、ESC/Enter 浮层栈与
 * `aria-modal` 语义。业务弹窗只提供内容与业务回调，不再自建外壳。
 *
 * 需要自定义尺寸/内衬时通过 `panelClassName` 传入 `modal-surface--*` 类族，
 * 不要在调用点重新拼 `modal-backdrop` / `modal-scrim` 结构。
 */
export function ModalOverlay({
  isOpen,
  onClose,
  ariaLabel,
  ariaLabelledBy,
  ariaDescribedBy,
  role = 'dialog',
  variant = 'modal',
  surface = 'glass',
  layer = 'base',
  panelClassName,
  backdropClassName,
  closeDisabled = false,
  priority = 0,
  onConfirm,
  onExitComplete,
  children,
}: ModalOverlayProps): ReactElement {
  const panelRef = useRef<HTMLDivElement>(null)
  const reducedMotion = useReducedMotion()
  const isDrawer = variant === 'drawer'

  useDismissStack(isOpen, onClose, { disabled: closeDisabled, priority, onConfirm })

  // 焦点约束与归还统一由共享 hook 提供，避免每个浮层各写一套导致漏项
  useFocusTrap(isOpen, panelRef)

  return (
    <AnimatePresence mode="wait" onExitComplete={onExitComplete} propagate>
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
          className={cn(
            'modal-backdrop cursor-pointer',
            isDrawer && 'modal-backdrop--drawer',
            LAYER_CLASS[layer],
            backdropClassName,
          )}
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
