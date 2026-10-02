/**
 * 数值输入草稿的纯逻辑：解析 / 收敛 / 格式化，不依赖 React。
 *
 * 三段式契约（见 .trellis/spec/web/frontend/component-guidelines.md 数值输入与校验时机）：
 * 1. 编辑期只维护字符串草稿，接受空串、`-`、`.`、不完整小数等键盘中间态；
 *    中间态一律不得写入提交模型（R4）。
 * 2. 失焦/回车时解析草稿并 clamp 到字段区间，再把结果派发给字段模型（R2）。
 * 3. 保存/应用路径对最终值再做一次 clamp，不假设 UI 已收敛（R3）。
 *
 * 参考实现：components/ui/NumericField.tsx（组件侧状态机）、
 * features/cameras/recordingConfig.ts#clampRecordingConfig（保存侧兜底）。
 */

export type NumericDraftType = 'number' | 'integer'

/**
 * 解析字符串草稿；返回 `null` 表示「还没有可提交的数值」。
 *
 * 键盘中间态（空串、`-`、`+`、`.`、`-.`、`+.`）与不可解析内容均返回 `null`，
 * 由调用方按字段语义决定回退还是落空值 —— 这些字符串不是「0」，也不是「清空」。
 */
export function parseNumericDraft(draft: string, type: NumericDraftType): number | null {
  const value = draft.trim()
  if (
    !value ||
    value === '-' ||
    value === '+' ||
    value === '.' ||
    value === '-.' ||
    value === '+.'
  ) {
    return null
  }

  if (type === 'integer' && !/^[+-]?\d+$/.test(value)) {
    return null
  }

  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : null
}

/** 把已解析的数值收敛到字段区间；非有限数（NaN / Infinity）安全回落到边界；缺省边界表示该侧不设限 */
export function clampNumericParam(value: number, minimum?: number, maximum?: number): number {
  if (!Number.isFinite(value)) {
    if (value === Number.POSITIVE_INFINITY && maximum !== undefined) return maximum
    if (minimum !== undefined) return minimum
    if (maximum !== undefined) return maximum
    return 0
  }
  const withMinimum = minimum === undefined ? value : Math.max(minimum, value)
  return maximum === undefined ? withMinimum : Math.min(maximum, withMinimum)
}

/** 把提交值格式化为显示草稿（整型字段取整，避免回写带小数点） */
export function formatNumericDraft(value: number, type: NumericDraftType): string {
  return type === 'integer' ? String(Math.round(value)) : String(value)
}

export function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}
