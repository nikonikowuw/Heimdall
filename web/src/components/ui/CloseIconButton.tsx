import type { ReactElement } from 'react'
import { X } from 'lucide-react'
import { cn } from '@/lib/utils'

export type CloseIconButtonVariant = 'plain' | 'surface'

export interface CloseIconButtonProps {
  onClick: () => void
  /** 本地化后的可访问名称；图标按钮必填，避免屏幕阅读器只读「按钮」 */
  label: string
  disabled?: boolean
  /** 保留键盘焦点但禁止操作，适用于提交期间的模态框控件 */
  ariaDisabled?: boolean
  title?: string
  variant?: CloseIconButtonVariant
  className?: string
}

const VARIANT_CLASS: Record<CloseIconButtonVariant, string> = {
  plain: 'text-[var(--text-muted)] hover:bg-[var(--bg-secondary)]',
  surface:
    'border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:bg-[var(--bg-secondary)]',
}

/**
 * 浮层右上角关闭按钮。
 *
 * 尺寸（36px）、圆角、聚焦环与禁用态在全仓保持一致；此前这组类名在 40+ 处
 * 被逐个复制，既产生视觉漂移（`--text-secondary` 与 `--text-muted` 混用），
 * 也让漏写 `aria-label` 成为默认结果。
 */
export function CloseIconButton({
  onClick,
  label,
  disabled = false,
  ariaDisabled = false,
  title,
  variant = 'plain',
  className,
}: CloseIconButtonProps): ReactElement {
  return (
    <button
      type="button"
      onClick={() => {
        if (!ariaDisabled) onClick()
      }}
      disabled={disabled}
      aria-disabled={ariaDisabled || undefined}
      aria-label={label}
      title={title}
      className={cn(
        'flex h-9 w-9 shrink-0 items-center justify-center rounded-xl transition-colors hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50 aria-disabled:cursor-not-allowed aria-disabled:opacity-50',
        VARIANT_CLASS[variant],
        className,
      )}
    >
      <X className="h-4 w-4" aria-hidden="true" />
    </button>
  )
}
