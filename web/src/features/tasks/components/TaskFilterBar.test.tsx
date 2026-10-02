// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ALGORITHM_FILTER_ALL } from '../taskFilter'
import { TaskFilterBar, type TaskFilterBarProps } from './TaskFilterBar'

// 模拟 react-i18next：返回 key 本身，便于断言文案键而非语言相关字面量
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    // common 域的清空名称与 task 域共用一个 mock
    i18n: { language: 'zh-CN' },
  }),
}))

afterEach(cleanup)

function makeProps(overrides: Partial<TaskFilterBarProps> = {}): TaskFilterBarProps {
  return {
    query: '',
    onQueryChange: vi.fn(),
    onQueryClear: vi.fn(),
    armStatus: 'all',
    onArmStatusChange: vi.fn(),
    armStatusCounts: { all: 30, armed: 12, disarmed: 18 },
    algorithmId: ALGORITHM_FILTER_ALL,
    onAlgorithmChange: vi.fn(),
    algorithmOptions: [
      { value: 'fire_detections', label: '火焰检测' },
      { value: 'general_detection', label: '通用检测' },
    ],
    onClearFilters: vi.fn(),
    matchedCount: 30,
    ...overrides,
  }
}

describe('TaskFilterBar', () => {
  it('reports query typing and clearing through separate callbacks', () => {
    const props = makeProps({ query: '东门' })
    render(<TaskFilterBar {...props} />)

    fireEvent.change(screen.getByRole('textbox'), { target: { value: '东门周' } })
    expect(props.onQueryChange).toHaveBeenCalledWith('东门周')

    fireEvent.click(screen.getByRole('button', { name: 'actions.clearSearch' }))
    expect(props.onQueryClear).toHaveBeenCalledTimes(1)
  })

  it('renders the three arm status buckets with their counts', () => {
    render(<TaskFilterBar {...makeProps()} />)

    const group = screen.getByRole('group', { name: 'filter.armStatusLabel' })
    expect(group).toBeTruthy()
    expect(screen.getByRole('button', { name: /filter\.armAll/ }).textContent).toContain('(30)')
    expect(screen.getByRole('button', { name: /filter\.armed/ }).textContent).toContain('(12)')
    expect(screen.getByRole('button', { name: /filter\.disarmed/ }).textContent).toContain('(18)')
  })

  it('reflects the selected bucket via aria-pressed and reports clicks', () => {
    const props = makeProps({ armStatus: 'armed' })
    render(<TaskFilterBar {...props} />)

    expect(screen.getByRole('button', { name: /filter\.armed/ }).getAttribute('aria-pressed')).toBe(
      'true',
    )
    expect(
      screen.getByRole('button', { name: /filter\.armAll/ }).getAttribute('aria-pressed'),
    ).toBe('false')

    fireEvent.click(screen.getByRole('button', { name: /filter\.disarmed/ }))
    expect(props.onArmStatusChange).toHaveBeenCalledWith('disarmed')
  })

  it('keeps the current bucket selected when it is clicked again', () => {
    const props = makeProps({ armStatus: 'armed' })
    render(<TaskFilterBar {...props} />)

    fireEvent.click(screen.getByRole('button', { name: /filter\.armed/ }))
    // 与 CamerasPage 一致：单击只上报被点的档位，由父层保持选中，不回退「全部」
    expect(props.onArmStatusChange).toHaveBeenCalledWith('armed')
    expect(props.onArmStatusChange).not.toHaveBeenCalledWith('all')
  })

  it('renders the all option plus every derived algorithm option', () => {
    render(<TaskFilterBar {...makeProps()} />)

    const select = screen.getByRole('combobox', { name: 'filter.algorithmLabel' })
    expect(
      [...select.querySelectorAll('option')].map((option) => option.getAttribute('value')),
    ).toEqual([ALGORITHM_FILTER_ALL, 'fire_detections', 'general_detection'])
    expect(select.textContent).toContain('火焰检测')
  })

  it('reports algorithm changes through the narrowed value', () => {
    const props = makeProps()
    render(<TaskFilterBar {...props} />)

    fireEvent.change(screen.getByRole('combobox', { name: 'filter.algorithmLabel' }), {
      target: { value: 'fire_detections' },
    })
    expect(props.onAlgorithmChange).toHaveBeenCalledWith('fire_detections')
  })

  it('shows the clear action only when a filter is active', () => {
    const { rerender } = render(<TaskFilterBar {...makeProps()} />)
    expect(screen.queryByRole('button', { name: /filter\.clearAll/ })).toBeNull()

    // 生效筛选由组件从受控值自行推导，无需调用方额外传入开关
    const props = makeProps({ query: '东门' })
    rerender(<TaskFilterBar {...props} />)
    fireEvent.click(screen.getByRole('button', { name: /filter\.clearAll/ }))
    expect(props.onClearFilters).toHaveBeenCalledTimes(1)
  })

  it('shows the matched count only when it diverges from the full total', () => {
    const { rerender } = render(<TaskFilterBar {...makeProps()} />)
    expect(screen.queryByText('filter.matched')).toBeNull()

    rerender(<TaskFilterBar {...makeProps({ matchedCount: 3 })} />)
    expect(screen.getByText('filter.matched')).toBeTruthy()
  })
})
