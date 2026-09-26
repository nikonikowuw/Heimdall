import type { ChangeEvent, ReactElement, ReactNode, SelectHTMLAttributes } from 'react'
import { ChevronDown, type LucideIcon } from 'lucide-react'
import { cn } from '@/lib/utils'
import { narrowSelectValue, type SelectOptionLike } from '@/lib/unionNarrowing'

/** 下拉选项：`label` 为可见文案，`value` 即收窄后返回的字面量 */
export interface SelectFieldOption<T extends string> extends SelectOptionLike<T> {
  label: string
  disabled?: boolean
}
export type SelectFieldSize = 'default' | 'compact'

export interface SelectFieldProps<T extends string> extends Omit<
  SelectHTMLAttributes<HTMLSelectElement>,
  'onChange' | 'value' | 'size' | 'children' | 'aria-label'
> {
  value: T
  /** 收窄后的取值回调；非法值不会到达此处，无需调用方再做类型断言 */
  onChange: (value: T, event: ChangeEvent<HTMLSelectElement>) => void
  /**
   * 可访问名称。设为必填，让漏写 `aria-label` 从运行期缺陷变成编译期错误；
   * 通常传入与该筛选维度一致的文案（如「全部通道」）。
   */
  label: string
  /**
   * 候选选项。既是渲染来源，也是运行时收窄依据 —— 原生 `<select>` 的
   * `event.target.value` 只会给出 `string`，与 `options` 比对即可安全还原为 `T`，
   * 因此调用方无需再写 `as` 断言或 `isMemberOf` 校验。
   */
  options: ReadonlyArray<SelectFieldOption<T>>
  /** 可选的「全部 / 不限」档，渲染在列表首位 */
  allOption?: SelectFieldOption<T>
  /** 左侧修饰图标，用于提示筛选维度 */
  icon?: LucideIcon
  /**
   * 强调态：该维度已收敛当前视图（值不再等于「全部」）时高亮。
   * 由组件统一取值，避免每个调用点各写一套条件模板串。
   */
  emphasis?: boolean
  sizeVariant?: SelectFieldSize
  /** 容器类名（宽度、flex 权重等布局由调用方决定） */
  containerClassName?: string
}

/**
 * 工具栏与表单通用的下拉选择控件。
 *
 * 与 [SearchInput](./SearchInput.tsx) 同源：高度、圆角与聚焦环对齐，差异仅在
 * 右侧是 caret 而非清空按钮。承担三件容易遗漏的事：
 *
 * 1. **可访问名称**：`label` 必填并落到 `aria-label`；
 * 2. **可见焦点**：`.select-field:focus-visible` 在双主题下提供焦点环，
 * 替代裸 `outline-none` 造成的键盘焦点隐形；
 * 3. **外观一致**：重置原生 `appearance` 后渲染主题化 caret，避免各平台原生
 * 箭头与工具栏内其他控件风格冲突。
 *
 * 外观需要强调「筛选已生效」（accent 描边胶囊）等特殊形态时，通过 `className`
 * 覆盖：`.select-field` 声明在 `@layer components`，utilities 可正常压过它。
 * 确实需要完全自定义渲染时才退回裸 `<select>`，但必须自行补 `aria-label`
 * 与 `focus-visible` 样式。
 */
export function SelectField<T extends string>({
  value,
  onChange,
  label,
  options,
  allOption,
  icon: Icon,
  emphasis = false,
  sizeVariant = 'default',
  containerClassName,
  className,
  disabled = false,
  ...restProps
}: SelectFieldProps<T>): ReactElement {
  /*
   * 当前值不在选项表内时必须补进去。
   *
   * 调用方的 `value` 常与动态收敛出的 `options` 配对（如候选只来自当前页已出现的
   * 标签），而这些选项会随筛选、翻页、重连变化。若此时不补，浏览器会选中首项或
   * 渲染成空白，而父层 state 仍带着旧值继续过滤：**屏幕上展示的与请求实际生效的
   * 不再是同一个值**，用户也无从看到或清掉这个隐形筛选。入口列表与展示状态必须一致。
   */
  const isKnown = value === allOption?.value || options.some((option) => option.value === value)

  const items: ReactNode = (
    <>
      {allOption ? <option value={allOption.value}>{allOption.label}</option> : null}
      {options.map((option) => (
        <option key={option.value} value={option.value} disabled={option.disabled}>
          {option.label}
        </option>
      ))}
      {isKnown ? null : <option value={value}>{value}</option>}
    </>
  )

  /**
   * 把原生 `string` 收窄回 `T`，非法值直接丢弃。
   * 收窄规则见 [narrowSelectValue](#narrowselectvalue)，调用方因此无需 `as` 断言。
   */
  const handleChange = (event: ChangeEvent<HTMLSelectElement>): void => {
    const matched = narrowSelectValue(event.target.value, options, allOption)
    if (matched === undefined) return
    onChange(matched, event)
  }

  return (
    <span
      className={cn(
        'select-field-wrap',
        Icon && 'select-field-wrap--with-icon',
        emphasis && 'select-field-wrap--emphasis',
        containerClassName,
      )}
    >
      {Icon ? <Icon className="select-field__icon" aria-hidden="true" /> : null}
      <select
        value={value}
        disabled={disabled}
        aria-label={label}
        onChange={handleChange}
        className={cn(
          'select-field',
          sizeVariant === 'compact' && 'select-field--compact',
          emphasis && 'select-field--emphasis',
          className,
        )}
        {...restProps}
      >
        {items}
      </select>
      <ChevronDown className="select-field__caret" aria-hidden="true" />
    </span>
  )
}
