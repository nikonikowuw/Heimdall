import { forwardRef, type ChangeEvent, type InputHTMLAttributes, type ReactElement } from 'react'
import { Search, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'

export type SearchInputSize = 'default' | 'compact' | 'form'

export interface SearchInputProps extends Omit<
  InputHTMLAttributes<HTMLInputElement>,
  'size' | 'onChange'
> {
  /** 当前输入值 */
  value: string
  /** 输入值变化回调 */
  onChange: (value: string, event: ChangeEvent<HTMLInputElement>) => void
  /** 清空回调：提供且 value 非空时展示清空按钮 */
  onClear?: () => void
  /** 尺寸变体：默认 36px (default)、紧凑 32px (compact)、表单 40px (form) */
  sizeVariant?: SearchInputSize
  /** 是否展示全局快捷键 '/' 聚焦提示徽章（仅在 value 为空时展示） */
  showKbdHint?: boolean
  /** 清空按钮无障碍说明与悬浮提示，默认 '清空搜索' */
  clearAriaLabel?: string
  /** 清空按钮附加样式（如自定义光标目标 reticle-target） */
  clearButtonClassName?: string
  /** 外部容器类名 */
  containerClassName?: string
}

/**
 * 全站通用的搜索输入框规范组件。
 *
 * 封装 .search-field 结构体系，内置：
 * - 搜索图标（固定垂直居中与主题高亮过渡）
 * - 清空按钮（自动关联无障碍 label、title 与键盘焦点）
 * - 快捷键 '/' 提示徽章（响应式隐藏）
 * - `data-search-input` 标记（供 [use-global-shortcuts](../hooks/use-global-shortcuts.ts) 定位；
 * 由组件统一输出，调用点不必逐个声明）
 * - 兼容 compact / form / default 尺寸与原生 ref 转发
 */
export const SearchInput = forwardRef<HTMLInputElement, SearchInputProps>(function SearchInput(
  {
    value,
    onChange,
    onClear,
    sizeVariant = 'default',
    showKbdHint = false,
    clearAriaLabel,
    clearButtonClassName,
    containerClassName,
    className,
    disabled = false,
    placeholder,
    'aria-label': ariaLabel,
    type = 'text',
    ...restProps
  },
  ref,
): ReactElement {
  const { t } = useTranslation()
  const resolvedClearLabel =
    clearAriaLabel ?? t('actions.clearSearch', { defaultValue: '清空搜索' })
  const resolvedAriaLabel = ariaLabel ?? placeholder

  return (
    <div
      className={cn(
        'search-field',
        sizeVariant === 'compact' && 'search-field--compact',
        sizeVariant === 'form' && 'search-field--form',
        containerClassName,
      )}
    >
      <Search className="search-field__icon" aria-hidden="true" />
      <input
        ref={ref}
        type={type}
        value={value}
        onChange={(event) => onChange(event.target.value, event)}
        disabled={disabled}
        placeholder={placeholder}
        aria-label={resolvedAriaLabel}
        data-search-input="true"
        className={cn('search-field__input', className)}
        {...restProps}
      />
      {value && onClear ? (
        <button
          type="button"
          onClick={onClear}
          disabled={disabled}
          aria-label={resolvedClearLabel}
          title={resolvedClearLabel}
          className={cn('search-field__clear', clearButtonClassName)}
        >
          <X className={sizeVariant === 'compact' ? 'h-3 w-3' : 'h-3.5 w-3.5'} aria-hidden="true" />
        </button>
      ) : showKbdHint ? (
        <kbd className="search-field__hint hidden sm:inline-flex">/</kbd>
      ) : null}
    </div>
  )
})
