import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { renderToString } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Drawer, type DrawerSize } from './Drawer'

const CSS = readFileSync(join(process.cwd(), 'src/styles/globals.css'), 'utf8')

/** 取第一条选择器规则体；找不到时返回空串，让断言直接失败而非静默通过 */
function cssRule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  return CSS.match(new RegExp(`\\n\\s*${escaped}\\s*\\{([^}]*)\\}`))?.[1] ?? ''
}

function renderDrawer(size: DrawerSize = 'medium', closeDisabled = false): string {
  return renderToString(
    <Drawer
      isOpen
      onClose={() => {}}
      closeLabel="Close drawer"
      title="Drawer title"
      description="Drawer description"
      icon={<span aria-hidden="true">I</span>}
      metadata={<span>Metadata</span>}
      headerActions={<button type="button">Header action</button>}
      toolbar={<span>Toolbar content</span>}
      footer={<span>Footer content</span>}
      size={size}
      closeDisabled={closeDisabled}
    >
      <p>Body content</p>
    </Drawer>,
  )
}

describe('Drawer', () => {
  it.each([
    ['small', 'modal-surface--small'],
    ['compact', 'modal-surface--compact'],
    ['medium', 'modal-surface--drawer-medium'],
    ['wide', 'modal-surface--drawer-wide'],
  ] as const)('uses the solid drawer surface and %s width', (size, widthClass) => {
    const html = renderDrawer(size)

    expect(html).toContain('modal-backdrop--drawer')
    expect(html).toContain(widthClass)
    expect(html).not.toContain('modal-surface--glass')
  })

  it('renders labelled title and description with shared structural regions', () => {
    const html = renderDrawer()
    const titleId = html.match(/aria-labelledby="([^"]+)"/)?.[1]
    const descriptionId = html.match(/aria-describedby="([^"]+)"/)?.[1]

    expect(titleId).toBeTruthy()
    expect(descriptionId).toBeTruthy()
    expect(html).toContain(`id="${titleId}"`)
    expect(html).toContain(`id="${descriptionId}"`)
    expect(html).toContain('drawer-header')
    expect(html).toContain('drawer-toolbar')
    expect(html).toContain('drawer-body')
    expect(html).toContain('drawer-footer')
    expect(html).toContain('aria-label="Close drawer"')
  })

  it('keeps the close control focusable but disabled during an operation', () => {
    const html = renderDrawer('medium', true)
    const closeButton = html.match(/<button[^>]*aria-label="Close drawer"[^>]*>/)?.[0]

    expect(closeButton).toBeDefined()
    expect(closeButton).toContain('aria-disabled="true"')
    expect(closeButton).not.toMatch(/\sdisabled(?:=|\s|>)/)
  })

  it('carries an optional native tooltip on the title node', () => {
    const html = renderToString(
      <Drawer
        isOpen
        onClose={() => {}}
        closeLabel="Close drawer"
        title="Very long device name"
        titleTooltip="Very long device name"
      >
        <p>Body content</p>
      </Drawer>,
    )

    expect(html).toContain('title="Very long device name"')
  })
})

describe('Drawer structural contract', () => {
  // 头/工具栏/底栏固定、主体唯一滚动：契约写在 globals.css，SSR 字符串断言
  // 无法覆盖，因此直接校验样式规则，防止后续重构悄悄把滚动职责挪回外层。
  it('keeps header, toolbar and footer fixed while only the body scrolls', () => {
    const body = cssRule('.drawer-body')

    expect(body).toContain('flex: 1 1 auto')
    expect(body).toContain('min-height: 0')
    expect(body).toContain('overflow-y: auto')
    expect(body).toContain('padding: 1.25rem')

    for (const selector of ['.drawer-header', '.drawer-toolbar', '.drawer-footer']) {
      expect(cssRule(selector), selector).toContain('flex: 0 0 auto')
    }
  })

  it('right-aligns a lone footer action so call sites need no flex wrapper', () => {
    expect(cssRule('.drawer-footer > :only-child')).toContain('margin-inline-start: auto')
  })
})
