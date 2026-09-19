import { renderToString } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { Camera } from '@/types'
import { CameraDetailDrawer } from './CameraDetailDrawer'

// 模拟 react-i18next
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue || key,
    i18n: { language: 'zh-CN' },
  }),
}))

const mockCamera: Camera = {
  id: 1,
  cameraId: 'cam_001',
  name: '东门周界枪机',
  protocol: 'rtsp',
  rtspUrl: 'rtsp://192.168.1.100:554/live/main',
  subRtspUrl: 'rtsp://192.168.1.100:554/live/sub',
  streamMode: 'auto',
  remark: '1号大门主通道',
  lastProbeStatus: 'healthy',
  lastProbeAt: 1710000000000,
  lastCodec: 'hevc',
  lastWidth: 3840,
  lastHeight: 2160,
  lastFps: 30,
  createdAt: 1700000000000,
  updatedAt: 1700000000000,
}

describe('CameraDetailDrawer', () => {
  it('returns null when camera is null', () => {
    const html = renderToString(<CameraDetailDrawer camera={null} onClose={() => {}} />)
    expect(html).toBe('')
  })

  it('renders complete camera details including main and sub stream URLs', () => {
    const html = renderToString(<CameraDetailDrawer camera={mockCamera} onClose={() => {}} />)

    expect(html).toContain('东门周界枪机')
    expect(html).toContain('cam_001')
    expect(html).toContain('rtsp://192.168.1.100:554/live/main')
    expect(html).toContain('rtsp://192.168.1.100:554/live/sub')
    expect(html).toContain('hevc')
    expect(html).toContain('3840×2160')
    expect(html).toContain('30 FPS')
    expect(html).toContain('1号大门主通道')
  })

  it('displays hint when sub stream is not configured', () => {
    const singleStreamCam: Camera = {
      ...mockCamera,
      subRtspUrl: '',
    }
    const html = renderToString(<CameraDetailDrawer camera={singleStreamCam} onClose={() => {}} />)
    expect(html).toContain('未配置子码流')
  })

  it('renders action buttons and model selector options', () => {
    const html = renderToString(
      <CameraDetailDrawer
        camera={mockCamera}
        onClose={() => {}}
        onManualProbe={() => {}}
        onEdit={() => {}}
        onDelete={() => {}}
      />,
    )

    expect(html).toContain('筒形枪机')
    expect(html).toContain('半球摄像机')
    expect(html).toContain('云台球机')
    expect(html).toContain('actions.edit')
    expect(html).toContain('actions.delete')
  })
})
