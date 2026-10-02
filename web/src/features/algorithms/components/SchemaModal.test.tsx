// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import '@/i18n'
import type { AlgorithmItem, AlgorithmVersionItem } from '@/types'
import { SchemaModal } from './SchemaModal'

afterEach(cleanup)

function makeVersion(configSchema: Record<string, unknown>): AlgorithmVersionItem {
  return {
    id: 1,
    algorithmId: 'general_detection',
    version: '1.0.0',
    platformId: 'rk3588',
    normalizedPlatformId: 'rk3588',
    compatibleWithHost: true,
    minAdapterVersion: '0.1.0',
    packageRoot: 'general_detection',
    fpsTiers: [],
    configSchema,
    manifestRaw: {},
    packageSizeBytes: 1024,
    isActive: true,
    isBuiltin: false,
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
  }
}

function makeAlgorithm(configSchema: Record<string, unknown>, name = '通用检测'): AlgorithmItem {
  return {
    id: 1,
    algorithmId: 'general_detection',
    name,
    algorithmType: 'detection',
    alarmTypeId: 'intrusion',
    activeVersion: '1.0.0',
    description: '',
    isBuiltin: false,
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
    versions: [makeVersion(configSchema)],
  }
}

function renderModal(algorithm: AlgorithmItem | null = makeAlgorithm({})): HTMLElement {
  return render(<SchemaModal isOpen algorithm={algorithm} onClose={() => {}} />).container
}

describe('SchemaModal 结构契约', () => {
  it('复用全局浮层与表单弹窗原语，不自建外壳', () => {
    const container = renderModal()
    const dialog = screen.getByRole('dialog')
    const html = dialog.innerHTML

    expect(container.querySelector('.modal-backdrop')).toBeTruthy()
    expect(html).toContain('modal-form-header')
    expect(html).toContain('modal-form-toolbar')
    expect(html).toContain('modal-form-content')
    expect(html).toContain('modal-form-footer')
    expect(html).toContain('modal-form-button--secondary')
    // 非表单弹窗默认玻璃面；本弹窗必须走实体面
    expect(dialog.className).toContain('modal-surface--form')
    expect(dialog.className).toContain('modal-surface--medium')
    expect(dialog.className).not.toContain('modal-surface--glass')
    // 尺寸只走 token 类族，不再传裸 max-w-2xl
    expect(dialog.className).not.toContain('max-w-2xl')
  })

  it('关闭控件暴露本地化可访问名，且字形对辅助技术隐藏', () => {
    // 尺寸/命中区契约由共享原语自身守护（见 CloseIconButton.test.tsx），
    // 这里只验证用户与辅助技术可观察到的行为，不复述样式类名。
    const container = renderModal()
    const header = container.querySelector('.modal-form-header') as HTMLElement
    const closeButton = within(header).getByRole('button', { name: '关闭' })

    expect(closeButton.getAttribute('type')).toBe('button')
    // 仓库未装 jest-dom，不用 toHaveAttribute
    expect(closeButton.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true')
  })

  it('用可见标题节点关联无障碍名称，不依赖拼接的 aria-label', () => {
    const container = renderModal(makeAlgorithm({}, '通用检测'))
    const dialog = screen.getByRole('dialog')
    const labelId = dialog.getAttribute('aria-labelledby')
    const descriptionId = dialog.getAttribute('aria-describedby')

    expect(labelId).toBeTruthy()
    expect(descriptionId).toBeTruthy()
    // aria-labelledby 存在时 aria-label 不应渲染（重复传属死参数）
    expect(dialog.getAttribute('aria-label')).toBeNull()

    const titleNode = container.querySelector(`#${CSS.escape(labelId as string)}`)
    expect(titleNode?.textContent).toContain('通用检测')
    expect(container.querySelector(`#${CSS.escape(descriptionId as string)}`)).toBeTruthy()
  })
})

describe('SchemaModal 参数表格', () => {
  /** 8 项 > PROPERTY_SEARCH_THRESHOLD(6)，用于触发过滤框 */
  const schema = {
    properties: {
      score_threshold: { type: 'number', default: 0.4, description: '告警置信度' },
      track_enabled: { type: 'boolean', default: true, description: '是否启用跟踪' },
      p3: { type: 'number', default: 1, description: '参数三' },
      p4: { type: 'number', default: 2, description: '参数四' },
      p5: { type: 'number', default: 3, description: '参数五' },
      p6: { type: 'number', default: 4, description: '参数六' },
      p7: { type: 'number', default: 5, description: '参数七' },
      // 非对象条目，extractConfigProperties 应剔除
      broken: 42,
    },
  }

  it('从共享解析中取 properties，渲染声明的字段并丢弃畸形条目', () => {
    renderModal(makeAlgorithm(schema))

    expect(screen.getByText('score_threshold')).toBeDefined()
    expect(screen.getByText('track_enabled')).toBeDefined()
    expect(screen.getByText('0.4')).toBeDefined()
    expect(screen.getByText('告警置信度')).toBeDefined()
    // `broken: 42` 不是对象，extractConfigProperties 应已剔除
    expect(screen.queryByText('broken')).toBeNull()
  })

  it('按参数名或说明过滤表格；无命中时展示空态', () => {
    renderModal(makeAlgorithm(schema))
    const filter = screen.getByLabelText('按参数名或说明过滤...')

    fireEvent.change(filter, { target: { value: '跟踪' } })
    expect(screen.getByText('track_enabled')).toBeDefined()
    expect(screen.queryByText('score_threshold')).toBeNull()

    fireEvent.change(filter, { target: { value: '不存在的参数' } })
    expect(screen.getByText('没有匹配的参数项')).toBeDefined()
    expect(screen.queryByText('track_enabled')).toBeNull()
  })

  it('属性不超过阈值时不渲染过滤框，避免少数几项还多一层交互', () => {
    renderModal(makeAlgorithm({ properties: { a: { type: 'number' }, b: { type: 'string' } } }))

    expect(screen.queryByLabelText('按参数名或说明过滤...')).toBeNull()
  })

  it('无任何参数时展示空态而非空表格', () => {
    renderModal(makeAlgorithm({ properties: {} }))

    expect(screen.getByText('该算法无额外自定义配置参数')).toBeDefined()
  })

  it('切换到 JSON 视图展示原始 schema，且过滤只作用于表格视图', () => {
    renderModal(makeAlgorithm(schema))
    const filter = screen.getByLabelText('按参数名或说明过滤...')
    fireEvent.change(filter, { target: { value: '跟踪' } })

    fireEvent.click(screen.getByRole('button', { name: 'JSON Schema' }))

    // 原始 JSON 始终完整，不受过滤影响
    const raw = screen.getByText(/"score_threshold"/)
    expect(raw.textContent).toContain('"track_enabled"')
    expect(screen.queryByLabelText('按参数名或说明过滤...')).toBeNull()
  })
})

describe('SchemaModal 关闭行为', () => {
  it('关闭按钮与页脚按钮都回调 onClose', () => {
    const onClose = vi.fn()
    const container = render(
      <SchemaModal isOpen algorithm={makeAlgorithm({})} onClose={onClose} />,
    ).container

    const header = container.querySelector('.modal-form-header') as HTMLElement
    fireEvent.click(within(header).getByRole('button', { name: '关闭' }))
    expect(onClose).toHaveBeenCalledTimes(1)

    const footer = container.querySelector('.modal-form-footer') as HTMLElement
    fireEvent.click(within(footer).getByRole('button', { name: '关闭' }))
    expect(onClose).toHaveBeenCalledTimes(2)
  })

  it('isOpen 为 false 时不渲染浮层', () => {
    render(<SchemaModal isOpen={false} algorithm={makeAlgorithm({})} onClose={() => {}} />)

    expect(screen.queryByRole('dialog')).toBeNull()
  })
})
