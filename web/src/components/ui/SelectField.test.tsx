import { describe, expect, it } from 'vitest'
import { renderToString } from 'react-dom/server'
import { createElement } from 'react'
import { Cpu } from 'lucide-react'
import { SelectField, type SelectFieldProps } from './SelectField'

const OPTIONS = [
  { value: 'all', label: '全部' },
  { value: 'roi', label: '区域入侵' },
] as const

type RuleType = (typeof OPTIONS)[number]['value']

/**
 * 渲染受测控件。`onChange` 在断言里从不触发，由助手统一提供，让每个用例只写
 * 自己真正关心的那两个属性。需要脱离选项联合的用例（如越界值）显式给出 `T`。
 */
function renderField<T extends string>(props: Omit<SelectFieldProps<T>, 'onChange'>): string {
  return renderToString(createElement(SelectField<T>, { ...props, onChange: () => {} }))
}

describe('SelectField', () => {
  it('renders a native select carrying the required accessible name', () => {
    const html = renderField<RuleType>({ label: '规则类型', value: 'all', options: OPTIONS })

    expect(html).toContain('<select')
    expect(html).toContain('aria-label="规则类型"')
    expect(html).toContain('select-field')
  })

  it('renders the all-option ahead of the declared options', () => {
    const html = renderField<string>({
      label: '目标类别',
      value: '',
      allOption: { value: '', label: '不限' },
      options: [{ value: 'person', label: '人员' }],
    })

    expect(html.indexOf('不限')).toBeGreaterThanOrEqual(0)
    expect(html.indexOf('人员')).toBeGreaterThan(html.indexOf('不限'))
  })

  it('keeps an out-of-band value selectable when options are derived from the current page', () => {
    // 候选来自「当前页已出现的标签」时，已生效的筛选值可能已不在候选内。
    // 若不为它补一个 option，浏览器会渲染成空白/首项，而父层 state 仍按该值过滤，
    // 屏幕上展示的与请求实际生效的将不再是同一个值。
    const html = renderField<string>({
      label: '目标类别',
      value: 'dog',
      allOption: { value: '', label: '不限' },
      options: [{ value: 'person', label: '人员' }],
    })

    expect(html).toContain('<option value="dog" selected="">dog</option>')
    // 渲染时的选中项必须与传入的 value 一致，否则展示与生效状态背离
    expect(html.match(/selected/g)).toHaveLength(1)
  })

  it('does not duplicate the current value when it is already an option', () => {
    const html = renderField<RuleType>({ label: '规则类型', value: 'roi', options: OPTIONS })

    expect(html.match(/value="roi"/g)).toHaveLength(1)
  })

  it('always renders the themed caret instead of relying on the native arrow', () => {
    const html = renderField<RuleType>({ label: '排序', value: 'all', options: OPTIONS })

    expect(html).toContain('select-field__caret')
    expect(html).toContain('aria-hidden="true"')
  })

  it('renders the icon variant with its layout modifier', () => {
    const html = renderField<RuleType>({ label: '平台', value: 'all', options: OPTIONS, icon: Cpu })

    expect(html).toContain('select-field-wrap--with-icon')
    expect(html).toContain('select-field__icon')
  })

  it('marks the emphasised state when the dimension is narrowing the view', () => {
    const html = renderField<RuleType>({
      label: '排序',
      value: 'roi',
      options: OPTIONS,
      emphasis: true,
    })

    expect(html).toContain('select-field--emphasis')
    expect(html).toContain('select-field-wrap--emphasis')
  })

  it('omits the emphasis modifier when inactive', () => {
    const html = renderField<RuleType>({ label: '排序', value: 'all', options: OPTIONS })

    expect(html).not.toContain('select-field--emphasis')
  })

  it('applies the compact size modifier', () => {
    const html = renderField<RuleType>({
      label: '模块',
      value: 'all',
      options: OPTIONS,
      sizeVariant: 'compact',
    })

    expect(html).toContain('select-field--compact')
  })

  it('forwards container, id and title without dropping layout classes', () => {
    const html = renderField<RuleType>({
      label: '通道',
      value: 'all',
      options: OPTIONS,
      containerClassName: 'min-w-40 flex-1',
      title: '通道筛选',
      id: 'channel-filter',
    })

    expect(html).toContain('min-w-40')
    expect(html).toContain('title="通道筛选"')
    expect(html).toContain('id="channel-filter"')
  })

  it('marks the select disabled when requested', () => {
    const html = renderField<RuleType>({
      label: '排序',
      value: 'all',
      options: OPTIONS,
      disabled: true,
    })

    expect(html).toContain('disabled=""')
  })
})
