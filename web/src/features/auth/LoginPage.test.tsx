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

vi.mock('@/lib/api', () => ({
  authApi: {
    getInitStatus: vi.fn(() => Promise.resolve({ initialized: true })),
    initialize: vi.fn(),
    login: vi.fn(),
  },
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
