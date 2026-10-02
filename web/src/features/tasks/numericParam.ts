/**
 * 算法参数 JSON Schema → 数值参数配置。
 *
 * 只保留 schema 语义（类型/区间/默认值）；草稿解析、区间收敛与格式化等
 * 与 UI 交互相关的逻辑在同名纯逻辑模块 [numericDraft](../../lib/numericDraft.ts) 中，
 * 由 [AlgoParamDrawer](./components/AlgoParamDrawer.tsx) 直接消费。
 */

import { isFiniteNumber, type NumericDraftType } from '@/lib/numericDraft'

/** 算法参数的数值语义，与 `NumericDraftType` 同源 */
export type NumericParamType = NumericDraftType

export interface NumericParamConfig {
  type: NumericParamType
  minimum?: number
  maximum?: number
  defaultValue: number
}

function asFiniteNumber(value: unknown): number | undefined {
  return isFiniteNumber(value) ? value : undefined
}

/**
 * 从 JSON Schema 属性推导数值参数配置。
 *
 * 未声明 `maximum` 的字段必须保持无上界（历史缺陷：默认成 1，把自由参数压成布尔量）。
 */
export function getNumericParamConfig(
  property: Record<string, unknown>,
): NumericParamConfig | null {
  const type = property.type
  if (type !== 'number' && type !== 'integer') return null

  const minimum = asFiniteNumber(property.minimum)
  const maximum = asFiniteNumber(property.maximum)
  const defaultValue = asFiniteNumber(property.default) ?? minimum ?? 0

  return { type, minimum, maximum, defaultValue }
}
