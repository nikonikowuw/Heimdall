import type { ReactElement } from 'react'
import { AlertCircle } from 'lucide-react'
import { cn } from '@/lib/utils'

export interface FormErrorAlertProps {
  /** 错误文本；为空时不渲染，调用点无需再包一层条件 */
  message: string | null | undefined
  /** 是否在图标下方对齐多行文本，用于说明较长的错误 */
  alignTop?: boolean
  className?: string
}

/**
 * 表单/弹窗内联错误提示。
 *
 * 统一「危险语义描边 + 柔和底 + 危险色文本」的呈现，避免每个提交类弹窗
 * 各自拼一套配色；错误文本的取用与本地化由调用方负责（见错误处理规范：
 * Toast 与字段错误只由一处负责）。
 */
export function FormErrorAlert({
  message,
  alignTop = false,
  className,
}: FormErrorAlertProps): ReactElement | null {
  if (!message) return null

  return (
    <div
      role="alert"
      className={cn(
        'border-status-danger-border bg-status-danger-soft text-status-danger flex gap-2 rounded-xl border p-3 text-xs',
        alignTop ? 'items-start' : 'items-center',
        className,
      )}
    >
      <AlertCircle className={cn('h-4 w-4 shrink-0', alignTop && 'mt-0.5')} aria-hidden="true" />
      <span className="leading-relaxed">{message}</span>
    </div>
  )
}
