import { describe, expect, it } from 'vitest'
import { renderToString } from 'react-dom/server'
import { Video } from 'lucide-react'
import { PageHeader } from './PageHeader'

/** 页面标题栏的标准几何契约：改这里就等于改全局基准，必须同步核对所有页面 */
const HEIGHT_CONTRACT = 'min-h-[68px]'
const HEIGHT_CONTRACT_DESKTOP = 'sm:h-[68px]'
const HORIZONTAL_PADDING = 'px-4'
const ICON_BOX = 'h-10 w-10'
const ICON_GLYPH = 'h-5 w-5'
const SHELL = 'frosted-glass'

describe('PageHeader', () => {
  it('renders default semantic header with title and subtitle', () => {
    const html = renderToString(<PageHeader title="测试页面" subtitle={<span>副标题描述</span>} />)

    expect(html).toContain('<header')
    expect(html).toContain('测试页面')
    expect(html).toContain('副标题描述')
  })

  it('pins the height and padding contract so tabs cannot drift apart', () => {
    const html = renderToString(<PageHeader icon={Video} title="设备管理" subtitle="副标题" />)

    expect(html).toContain(HEIGHT_CONTRACT)
    expect(html).toContain(HEIGHT_CONTRACT_DESKTOP)
    expect(html).toContain(HORIZONTAL_PADDING)
  })

  it('keeps a constant height with or without a subtitle', () => {
    const withSubtitle = renderToString(<PageHeader icon={Video} title="A" subtitle="B" />)
    const withoutSubtitle = renderToString(<PageHeader icon={Video} title="A" />)

    // 高度契约必须在两种形态下同时成立，否则切换 Tab 时标题栏会跳动
    for (const html of [withSubtitle, withoutSubtitle]) {
      expect(html).toContain(HEIGHT_CONTRACT)
      expect(html).toContain(SHELL)
    }
  })

  it('locks the icon box and glyph to a single size', () => {
    const html = renderToString(<PageHeader icon={Video} title="设备管理" />)

    expect(html).toContain(ICON_BOX)
    expect(html).toContain(ICON_GLYPH)
    expect(html).toContain('lucide-video')
  })

  it('never wraps the action slot, which would grow the header height', () => {
    const html = renderToString(
      <PageHeader icon={Video} title="设备管理" actions={<button type="button">刷新</button>} />,
    )

    // actions 容器一旦允许换行，窄视口下按钮换行会把标题栏撑高
    const actionsWrapper = html.slice(html.lastIndexOf('<div'))
    expect(actionsWrapper).toContain('shrink-0')
    expect(actionsWrapper).not.toContain('flex-wrap')
  })

  it('supports custom ReactNode icon and iconIndicator', () => {
    const html = renderToString(
      <PageHeader
        icon={<span data-testid="custom-icon">CUSTOM</span>}
        iconIndicator={<span data-testid="status-dot">DOT</span>}
        title="带指示器的页面"
      />,
    )

    expect(html).toContain('data-testid="custom-icon"')
    expect(html).toContain('data-testid="status-dot"')
  })

  it('renders badges and actions slots', () => {
    const html = renderToString(
      <PageHeader
        title="监控控制台"
        badges={<span data-testid="badge-hw">硬件就绪</span>}
        actions={<button type="button">刷新</button>}
      />,
    )

    expect(html).toContain('data-testid="badge-hw"')
    expect(html).toContain('<button type="button">刷新</button>')
  })

  it('omits the icon shell when no icon is provided', () => {
    const html = renderToString(<PageHeader title="无图标页面" />)

    expect(html).not.toContain('relative flex h-10 w-10')
    expect(html).toContain('无图标页面')
  })
})
