import { describe, expect, it } from 'vitest'
import { getNumericParamConfig } from './numericParam'

// 草稿解析 / 区间收敛 / 格式化的用例已迁至 lib/numericDraft.test.ts，
// 本文件只覆盖 JSON Schema → 配置的推导语义。
describe('getNumericParamConfig', () => {
  it('未声明 maximum 时保持无上界，而不是默认成 1', () => {
    expect(getNumericParamConfig({ type: 'integer', minimum: 1, default: 30 })).toEqual({
      type: 'integer',
      minimum: 1,
      maximum: undefined,
      defaultValue: 30,
    })
  })

  it('非数值类型返回 null，交由其它控件渲染', () => {
    expect(getNumericParamConfig({ type: 'string' })).toBeNull()
    expect(getNumericParamConfig({ type: 'boolean' })).toBeNull()
    expect(getNumericParamConfig({})).toBeNull()
  })

  it('缺失 default 时依次回落到 minimum 与 0', () => {
    expect(getNumericParamConfig({ type: 'number', minimum: 0.5 })?.defaultValue).toBe(0.5)
    expect(getNumericParamConfig({ type: 'number' })?.defaultValue).toBe(0)
  })

  it('忽略非有限数值的区间声明', () => {
    expect(getNumericParamConfig({ type: 'number', minimum: Number.NaN })).toEqual({
      type: 'number',
      minimum: undefined,
      maximum: undefined,
      defaultValue: 0,
    })
  })
})
