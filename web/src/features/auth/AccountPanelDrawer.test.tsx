import { renderToString } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { AdminUserDto } from '@/types'
import { AccountPanelDrawer } from './AccountPanelDrawer'

// 模拟 react-i18next：返回 key 本身，便于断言文案键而非语言相关字面量
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue || key,
    i18n: { language: 'zh-CN' },
  }),
}))

// getMe 不参与首屏 SSR 渲染断言，避免真实网络依赖
vi.mock('@/lib/api', () => ({
  authApi: {
    getMe: vi.fn(() => Promise.resolve(mockUser)),
  },
}))

const mockUser: AdminUserDto = {
  username: 'operator-7',
  createdAt: 1710000000000,
  updatedAt: 1715000000000,
}

function render(open: boolean): string {
  return renderToString(
    <AccountPanelDrawer isOpen={open} onClose={() => {}} onOpenPasswordModal={() => {}} />,
  )
}

describe('AccountPanelDrawer', () => {
  it('renders nothing while closed', () => {
    expect(render(false)).toBe('')
  })

  it('exposes the account panel as a labelled modal drawer', () => {
    const html = render(true)

    expect(html).toContain('role="dialog"')
    expect(html).toContain('aria-modal="true"')
    expect(html).toContain('aria-label="accountPanel.title"')
  })

  it('renders the change-password entry and revoke notice inside the panel', () => {
    const html = render(true)

    // 改密入口属于用户域，随面板一起提供
    expect(html).toContain('changePassword')
    expect(html).toContain('revokeNotice')
  })

  it('does not hardcode a role label that bypasses i18n', () => {
    const html = render(true)

    // 回归保护：历史实现曾硬编码 "ROOT"，在中文界面漏出英文
    expect(html).not.toContain('>ROOT<')
  })
})
