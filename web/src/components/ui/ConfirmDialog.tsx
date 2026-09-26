import { useId, type ReactElement, type ReactNode } from 'react'
import { AlertTriangle, Loader2, type LucideIcon } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ModalOverlay } from './ModalOverlay'
import { FormErrorAlert } from './FormErrorAlert'
import { CloseIconButton } from './CloseIconButton'
import { cn } from '@/lib/utils'

export type ConfirmDialogVariant = 'danger' | 'warning'

export interface ConfirmDialogProps {
  isOpen: boolean
  title: string
  /** 影响范围说明；较长的不可逆后果描述放这里 */
  description?: string
  variant?: ConfirmDialogVariant
  /** 头部图标，缺省按 variant 取警告三角 */
  icon?: LucideIcon
  confirmLabel: string
  cancelLabel: string
  onConfirm: () => void
  onClose: () => void
  /** 确认中：禁用两个按钮与关闭，并显示旋转指示 */
  isConfirming?: boolean
  /** 提交失败原因，由调用点负责本地化与取用（Toast 与内联错误只由一处负责） */
  errorMessage?: string | null
  /** 需人工核对的对象信息、预览清单等追加内容 */
  children?: ReactNode
  /** 浮层栈优先级，嵌套在抽屉之上的确认弹窗传更大值 */
  priority?: number
  /** 显示「按 ↵ 确认 / ESC 关闭」键盘提示 */
  showKeyboardHint?: boolean
  className?: string
}

/**
 * 二次确认弹窗（危险/警告操作）。
 *
 * 外壳、焦点陷阱、ESC/Enter 语义统一由 [ModalOverlay](./ModalOverlay.tsx) 承担，
 * 本组件只负责「图标 + 标题 + 说明 + 可选核对内容 + 页脚操作」这套固定信息结构。
 *
 * 确认类弹窗通过 `onConfirm` 接管栈顶 Enter：焦点在输入框时让行，
 * 因此不影响调用点在弹窗内放置表单字段。
 */
export function ConfirmDialog({
  isOpen,
  title,
  description,
  variant = 'danger',
  icon,
  confirmLabel,
  cancelLabel,
  onConfirm,
  onClose,
  isConfirming = false,
  errorMessage,
  children,
  priority = 0,
  showKeyboardHint = false,
  className,
}: ConfirmDialogProps): ReactElement {
  const { t } = useTranslation('common')
  const titleId = useId()
  const descriptionId = useId()

  const Icon = icon ?? AlertTriangle
  const isDanger = variant === 'danger'

  return (
    <ModalOverlay
      isOpen={isOpen}
      onClose={onClose}
      ariaLabel={title}
      ariaLabelledBy={titleId}
      ariaDescribedBy={description ? descriptionId : undefined}
      role={isDanger ? 'alertdialog' : 'dialog'}
      surface="solid"
      closeDisabled={isConfirming}
      priority={priority}
      onConfirm={onConfirm}
      panelClassName={cn('modal-surface--compact', className)}
    >
      <div className="flex items-start gap-3.5">
        <div
          className={cn(
            'flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border shadow-xs',
            isDanger
              ? 'border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)]'
              : 'border-[var(--status-warning-border)] bg-[var(--status-warning-soft)] text-[var(--status-warning)]',
          )}
          aria-hidden="true"
        >
          <Icon className="h-5 w-5" />
        </div>

        <div className="min-w-0 flex-1">
          <h2
            id={titleId}
            className="text-base font-bold tracking-tight text-[var(--text-primary)]"
          >
            {title}
          </h2>
          {description && (
            <p
              id={descriptionId}
              className="mt-1 text-xs leading-relaxed text-[var(--text-secondary)]"
            >
              {description}
            </p>
          )}
        </div>

        <CloseIconButton onClick={onClose} label={t('close')} ariaDisabled={isConfirming} />
      </div>

      {children && <div className="mt-4 space-y-3">{children}</div>}

      {errorMessage && <FormErrorAlert message={errorMessage} alignTop className="mt-3" />}

      <div className="mt-6 flex items-center justify-between gap-3">
        {showKeyboardHint ? (
          <div className="hidden items-center gap-1 text-[11px] text-[var(--text-muted)] sm:flex">
            <span>{t('modal.enterHintPrefix')}</span>
            <kbd className="rounded border border-[var(--border)] bg-[var(--bg-secondary)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)] shadow-xs">
              ↵
            </kbd>
            <span>{t('modal.enterHintSuffix')}</span>
          </div>
        ) : (
          <span />
        )}

        <div className="flex items-center gap-2.5">
          <button
            type="button"
            onClick={() => {
              if (!isConfirming) onClose()
            }}
            aria-disabled={isConfirming || undefined}
            className="modal-form-button modal-form-button--secondary aria-disabled:cursor-not-allowed aria-disabled:opacity-50"
          >
            {cancelLabel}
          </button>
          <button
            type="button"
            data-autofocus
            onClick={() => {
              if (!isConfirming) onConfirm()
            }}
            aria-disabled={isConfirming || undefined}
            className={cn(
              'modal-form-button text-white aria-disabled:cursor-not-allowed aria-disabled:opacity-50',
              isDanger
                ? 'bg-[var(--status-danger-solid)] hover:bg-[var(--status-danger-solid)]/90'
                : 'bg-[var(--status-warning-solid)] hover:bg-[var(--status-warning-solid)]/90',
            )}
          >
            {isConfirming && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />}
            {confirmLabel}
          </button>
        </div>
      </div>
    </ModalOverlay>
  )
}
