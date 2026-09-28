import { renderToString } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { LoginPage } from './LoginPage'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'zh-CN' },
  }),
}))

vi.mock('@/components/LocaleDropdown', () => ({
  LocaleDropdown: () => null,
}))

// 这个 mock 必须完整覆盖 LoginPage 从 @/lib/api 导入的符号，否则模块装配即失败。
// 初始化探测的超时行为在未 mock 的 api.test.ts 中验证：本文件走 renderToString，
// 不执行 effect，因此无法观察任何提交后的交互行为。
vi.mock('@/lib/api', () => ({
  authApi: {
    getInitStatus: vi.fn(() => Promise.resolve({ initialized: true })),
    initialize: vi.fn(),
    login: vi.fn(),
  },
  RequestTimeoutError: class RequestTimeoutError extends Error {},
}))

describe('LoginPage', () => {
  it('keeps authentication controls hidden until initialization status is known', () => {
    const html = renderToString(<LoginPage />)

    expect(html).toContain('checkingTitle')
    expect(html).toContain('checkingStatus')
    expect(html).toContain('aria-busy="true"')
    expect(html).toMatch(/noValidate/i)
    expect(html).toContain('— FPS')
    expect(html).not.toContain('-- FPS')
    expect(html).not.toContain('id="username"')
    expect(html).not.toContain('id="password"')
    expect(html).not.toContain('type="submit"')
  })

  it('renders device-side zero-copy and hardware platform telemetry', () => {
    const html = renderToString(<LoginPage />)

    expect(html).toContain('zeroCopy')
    expect(html).toContain('deviceSide')
    expect(html).toContain('opticalSensor')
    expect(html).toContain('kerrMetric')
    expect(html).toContain('directBus')
  })
})
