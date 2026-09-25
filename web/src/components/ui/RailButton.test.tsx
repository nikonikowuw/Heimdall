import { renderToString } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { RailButton } from './RailButton'

function render(active = false): string {
  return renderToString(
    <RailButton
      active={active}
      onClick={() => {}}
      icon={<svg data-testid="icon" />}
      label="算法仓库"
    />,
  )
}

describe('RailButton', () => {
  it('exposes the label to assistive tech', () => {
    const html = render()

    // 图标按钮无可读文本，必须有稳定的可访问名，
    // 不能只依赖视觉浮层或原生 title（后者不可键盘触发）
    expect(html).toContain('sr-only')
    expect(html).toContain('算法仓库')
  })

  it('hides the visual tooltip from screen readers to avoid double announcement', () => {
    const html = render()

    // 浮层仅供视觉；可访问名由 sr-only 承担，两者必须只暴露一个
    expect(html).toMatch(/nav-tooltip[^>]*aria-hidden="true"/)
  })

  it('marks the active destination for assistive tech', () => {
    expect(render(true)).toContain('aria-current="page"')
    expect(render(false)).not.toContain('aria-current')
  })
})
