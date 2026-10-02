import { useState, type FocusEvent, type InputHTMLAttributes, type KeyboardEvent } from 'react'
import {
  clampNumericParam,
  formatNumericDraft,
  isFiniteNumber,
  parseNumericDraft,
  type NumericDraftType,
} from '@/lib/numericDraft'

/**
 * 数值输入字段的三段式契约载体。
 *
 * 1. 编辑期只维护字符串草稿并接受键盘中间态（空串、`-`、`.`、不完整小数），
 *    显示值永远来自草稿，不派发数值回调 —— 中间态不得写入提交模型。
 * 2. 失焦 / 回车时解析草稿、按字段语义收敛，再派发一次数值回调。
 * 3. 保存 / 应用路径仍须对最终值再 clamp 一次，不假设本组件已收敛。
 *
 * 契约与失焦语义表见 .trellis/spec/web/frontend/component-guidelines.md（数值输入与校验时机）；
 * 纯逻辑见 [numericDraft](../../lib/numericDraft.ts)。
 *
 * 为什么是 `type="text"`：`type="number"` 各浏览器对 `-`、`1.`、空串的 `value`
 * 归一化不一致（空串直接变 `''` 丢失编辑上下文，`-` 被丢弃），恰好会破坏本契约要
 * 保留的键盘中间态；原生 `min`/`max` 也不阻挡键入、只影响 `:invalid`，而站点均不在
 * `<form>` 提交路径上，实际从不触发浏览器校验。改用 `inputMode` 保留移动端数字键盘。
 * 不声明 `role="spinbutton"`：未实现方向键步进却宣告该角色会向辅助技术谎报能力。
 */
export type NumericFieldType = NumericDraftType

interface NumericFieldCommonProps extends Omit<
  InputHTMLAttributes<HTMLInputElement>,
  'value' | 'onChange' | 'onBlur' | 'onKeyDown' | 'type' | 'min' | 'max' | 'inputMode' | 'step'
> {
  /** 可访问名称；各站点标签形态差异大，组件只把它落到 `aria-label`，不内置 label 元素 */
  label: string
  /** 数值语义：`integer` 时草稿只接受整数字符串，收敛时取整 */
  type?: NumericFieldType
  /** 语义下界；不落到 DOM（原生 min/max 不阻挡键入），只在收敛时生效 */
  min?: number
  /** 语义上界；同上 */
  max?: number
}

/** 必填模式：空/非法草稿失焦回退上次合法值，回调只收 `number` */
export interface RequiredNumericFieldProps extends NumericFieldCommonProps {
  emptyBehavior?: 'revert'
  value: number
  onChange: (value: number) => void
}

/** 可空模式：空草稿失焦回调 `null` 且界面保持空白 */
export interface NullableNumericFieldProps extends NumericFieldCommonProps {
  emptyBehavior: 'null'
  value: number | null
  onChange: (value: number | null) => void
}

export type NumericFieldProps = RequiredNumericFieldProps | NullableNumericFieldProps

export function NumericField(props: NumericFieldProps): React.ReactElement {
  const {
    label,
    type = 'number',
    min,
    max,
    className,
    onChange,
    emptyBehavior,
    onFocus,
    ...inputProps
  } = props

  const formatValue = (next: number | null): string =>
    // 非有限外部值（NaN / Infinity）渲染不出有意义的数字：显示为空而不是字符串 'NaN'。
    // 站点侧也可能把 `Math.round(undefined * 100)` 之类的 NaN 传进来。
    isFiniteNumber(next) ? formatNumericDraft(next, type) : ''

  const [draft, setDraft] = useState(() =>
    // 初始草稿只做格式化，不 clamp：越界的既有值应在收敛前保持可见
    formatValue(props.value),
  )
  const [isEditing, setIsEditing] = useState(false)

  // 非编辑态同步外部值：用渲染期状态调整而非 effect。
  // effect 会先渲染一帧旧值再覆盖（外部值变化时表现为输入框闪一下），
  // 渲染期调整在本次渲染内直接收敛。编辑中刻意不同步，避免打断用户输入。
  //
  // `observedValue` 记录「上一次看到的外部值」而不是「上次提交值」：收敛完成后的显示
  // 不依赖父层回传（父层若不改值，草稿仍停在用户确认的数值，而不是被弹回）。
  // 比较用 `Object.is` 而非 `!==`：`NaN` 与自身不相等，用 `!==` 会让「外部值没变」
  // 判定永远为真，渲染期 setState 反复触发直到 React 抛 Too many re-renders。
  // `Object.is` 与 React 内部的状态比较语义一致（NaN 自反、+0/-0 不等）。
  const [observedValue, setObservedValue] = useState<number | null>(props.value)
  if (!Object.is(props.value, observedValue)) {
    setObservedValue(props.value)
    if (!isEditing) setDraft(formatValue(props.value))
  }

  const dispatch = (next: number | null): void => {
    if (emptyBehavior === 'null') {
      onChange(next)
    } else if (next !== null) {
      onChange(next)
    }
  }

  const commit = (raw: string): void => {
    if (raw.trim() === '') {
      if (emptyBehavior === 'null') {
        // 空串是清空意图：落 null，界面保持空白
        setDraft('')
        dispatch(null)
        return
      }
      // 必填字段清空不是合法值：回退上次合法值，不派发
      setDraft(formatValue(props.value))
      return
    }

    const parsed = parseNumericDraft(raw, type)
    if (parsed === null) {
      // 不可解析的非空串是笔误，不是清空意图：两种模式都回退
      setDraft(formatValue(props.value))
      return
    }

    const committed = clampNumericParam(parsed, min, max)
    // 原地格式化显示，不依赖父层回传，避免一帧回跳
    setDraft(formatValue(committed))
    dispatch(committed)
  }

  const handleFocus = (event: FocusEvent<HTMLInputElement>): void => {
    setIsEditing(true)
    onFocus?.(event)
  }

  const handleBlur = (event: FocusEvent<HTMLInputElement>): void => {
    setIsEditing(false)
    commit(event.currentTarget.value)
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>): void => {
    if (event.key === 'Enter') commit(event.currentTarget.value)
  }

  return (
    <input
      {...inputProps}
      type="text"
      inputMode={type === 'integer' ? 'numeric' : 'decimal'}
      autoComplete="off"
      aria-label={label}
      className={className}
      value={draft}
      onFocus={handleFocus}
      onChange={(event) => setDraft(event.currentTarget.value)}
      onBlur={handleBlur}
      onKeyDown={handleKeyDown}
    />
  )
}
