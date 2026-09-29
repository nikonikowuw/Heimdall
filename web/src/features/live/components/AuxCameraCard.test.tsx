import { renderToString } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { Camera } from '@/types'
import { AuxCameraCard } from './AuxCameraCard'

vi.mock('react-i18next', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-i18next')>()
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: { defaultValue?: string; name?: string; count?: number }) => {
        if (opts?.defaultValue) return opts.defaultValue
        if (opts?.name) return `${key}:${opts.name}`
        if (opts?.count !== undefined) return `${key}:${opts.count}`
        return key
      },
      i18n: { language: 'zh-CN' },
    }),
  }
})

vi.mock('@/components/LivePlayer', () => ({
  LivePlayer: (props: Record<string, unknown>) => (
    <div data-testid="live-player" data-camera-id={String(props.cameraId || '')} />
  ),
}))

vi.mock('../hooks/useCameraTelemetry', () => ({
  useCameraTelemetry: (cameraId: string) => ({
    cameraId,
    personCount: 5,
    carCount: 2,
    motionHeat: 0.8,
    timestamp: 1710000000000,
  }),
}))

const mockCamera: Camera = {
  id: 1,
  cameraId: 'cam_001',
  name: '北门周界主路枪机',
  protocol: 'rtsp',
  rtspUrl: 'rtsp://192.168.1.100:554/live/main',
  subRtspUrl: 'rtsp://192.168.1.100:554/live/sub',
  streamMode: 'auto',
  remark: '主要干道布防',
  lastProbeStatus: 'healthy',
  lastProbeAt: 1710000000000,
  lastCodec: 'h264',
  lastWidth: 1920,
  lastHeight: 1080,
  lastFps: 25,
  createdAt: 1700000000000,
  updatedAt: 1700000000000,
}

describe('AuxCameraCard 辅流卡片 UI 规范', () => {
  it('采用一体化画面内悬浮 HUD，不再向卡片下方堆叠易被挤压裁切的外置文本栏', () => {
    const html = renderToString(
      <AuxCameraCard
        camera={mockCamera}
        isFocused={false}
        isAlarming={false}
        onSelectHero={vi.fn()}
        onEditCamera={vi.fn()}
        onDeleteCamera={vi.fn()}
      />,
    )

    // 卡片本身锁定宽高比 aspect-video 并声明 on-dark-surface 深底子树
    expect(html).toContain('aspect-video')
    expect(html).toContain('on-dark-surface')
    expect(html).toContain('bg-[var(--video-surface)]')
    // 渲染机位名称与 1080P 分辨率徽章
    expect(html).toContain('北门周界主路枪机')
    expect(html).toContain('1080P')
    // 渲染悬浮实时遥测指标（人和车）与语义状态色
    expect(html).toContain('text-[var(--status-info)]')
    expect(html).toContain('text-[var(--status-warning)]')
    expect(html).toContain('5')
    expect(html).toContain('2')
    // 渲染在线状态
    expect(html).toContain('在线')
    expect(html).toContain('text-[var(--status-success)]')
    // 渲染后台 live-player 微缩视口
    expect(html).toContain('data-testid="live-player"')
  })

  it('支持 4K 分辨率徽章自适应计算', () => {
    const cam4k: Camera = {
      ...mockCamera,
      lastWidth: 3840,
      lastHeight: 2160,
    }
    const html = renderToString(
      <AuxCameraCard
        camera={cam4k}
        isFocused={false}
        onSelectHero={vi.fn()}
        onEditCamera={vi.fn()}
        onDeleteCamera={vi.fn()}
      />,
    )
    expect(html).toContain('4K')
  })

  it('在聚焦为主屏状态时展示休眠节能指引而非重复拉流', () => {
    const html = renderToString(
      <AuxCameraCard
        camera={mockCamera}
        isFocused={true}
        onSelectHero={vi.fn()}
        onEditCamera={vi.fn()}
        onDeleteCamera={vi.fn()}
      />,
    )
    expect(html).toContain('live.focusedOnHero')
    expect(html).toContain('live.auxStreamSleeping')
    // 休眠时辅流 LivePlayer 不挂载
    expect(html).not.toContain('data-testid="live-player"')
  })

  it('在突发告警时展示呼吸告警警示徽章与高亮红框', () => {
    const html = renderToString(
      <AuxCameraCard
        camera={mockCamera}
        isFocused={false}
        isAlarming={true}
        onSelectHero={vi.fn()}
        onEditCamera={vi.fn()}
        onDeleteCamera={vi.fn()}
      />,
    )
    expect(html).toContain('live.alarmDetected')
    expect(html).toContain('border-[var(--status-danger)]')
  })
})
