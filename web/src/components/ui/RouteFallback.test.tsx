import { renderToString } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { RouteFallback } from './RouteFallback'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'zh-CN' },
  }),
}))

describe('RouteFallback', () => {
  it('向辅助技术暴露进行中状态', () => {
    const html = renderToString(<RouteFallback />)

    expect(html).toContain('role="status"')
    expect(html).toContain('aria-busy="true"')
    expect(html).toContain('loading')
  })

  // `.route-fallback` 承载 200ms 延迟现身的意图：内网 chunk 通常数十毫秒到达，
  // 去掉该 class 会让占位在每次切换 tab 时闪烁。此处锁定该约束。
  it('使用延迟现身的占位样式以避免骨架闪烁', () => {
    expect(renderToString(<RouteFallback />)).toContain('route-fallback')
  })
})
