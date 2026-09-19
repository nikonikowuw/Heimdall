import { renderToString } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { ToastItemView, Toaster } from './Toast'
import { toast, useToastStore } from '@/stores/toast'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue || key,
    i18n: { language: 'zh-CN' },
  }),
}))

describe('ToastItemView', () => {
  it('renders title, message and accessibility attributes for success', () => {
    const html = renderToString(
      <ToastItemView
        item={{
          id: 'test-1',
          type: 'success',
          title: '探活成功',
          message: 'H.264 1080P 30FPS',
          category: '设备探活',
          duration: 4000,
          createdAt: Date.now(),
        }}
        onDismiss={() => {}}
      />,
    )

    expect(html).toContain('role="status"')
    expect(html).toContain('aria-live="polite"')
    expect(html).toContain('探活成功')
    expect(html).toContain('H.264 1080P 30FPS')
    expect(html).toContain('设备探活')
  })

  it('renders alert accessibility role for error notifications', () => {
    const html = renderToString(
      <ToastItemView
        item={{
          id: 'test-2',
          type: 'error',
          title: '连接超时',
          message: 'RTSP握手失败',
          duration: 4000,
          createdAt: Date.now(),
        }}
        onDismiss={() => {}}
      />,
    )

    expect(html).toContain('role="alert"')
    expect(html).toContain('aria-live="assertive"')
    expect(html).toContain('连接超时')
    expect(html).toContain('RTSP握手失败')
  })

  it('renders action button when action is provided', () => {
    const html = renderToString(
      <ToastItemView
        item={{
          id: 'test-3',
          type: 'warning',
          title: '证书即将过期',
          message: '还有3天到期',
          duration: 0,
          action: {
            label: '立即更新',
            onClick: () => {},
          },
          createdAt: Date.now(),
        }}
        onDismiss={() => {}}
      />,
    )

    expect(html).toContain('立即更新')
  })
})

describe('Toaster', () => {
  it('renders all active toasts from toast store', () => {
    useToastStore.getState().clearToasts()
    toast.success('Camera Connected')
    toast.error('Disk Full')

    const html = renderToString(<Toaster toasts={useToastStore.getState().toasts} />)
    expect(html).toContain('Camera Connected')
    expect(html).toContain('Disk Full')
    expect(html).toContain('role="region"')
    expect(html).toContain('aria-label="系统通知"')
  })
})
