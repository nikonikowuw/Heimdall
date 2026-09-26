import { describe, expect, it } from 'vitest'
import { renderToString } from 'react-dom/server'
import { SystemSettingsHeader } from './SystemSettingsHeader'

describe('SystemSettingsHeader', () => {
  it('renders shared title, description and actions without a card shell', () => {
    const html = renderToString(
      <SystemSettingsHeader
        title="网络/服务"
        description="管理工业边缘网络接口与 IP 配置"
        actions={<button type="button">刷新</button>}
      />,
    )

    expect(html).toContain('<header')
    expect(html).toContain('<h2')
    expect(html).toContain('网络/服务')
    expect(html).toContain('管理工业边缘网络接口与 IP 配置')
    expect(html).toContain('<button type="button">刷新</button>')
    expect(html).not.toContain('frosted-glass')
    expect(html).not.toContain('rounded-2xl')
  })
})
