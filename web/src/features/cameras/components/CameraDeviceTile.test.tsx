import { renderToString } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { Camera, TaskSummaryDto } from '@/types'
import { CameraDeviceTile } from './CameraDeviceTile'
import { getCameraTypeLabel } from './illustrations/cameraModelType'

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

const activeTask: TaskSummaryDto = {
  id: 1,
  cameraId: mockCamera.cameraId,
  name: '周界分析',
  desiredEnabled: true,
  actualStatus: 2,
  statusMessage: '',
  algorithmId: 'general_detection',
  analysisFps: 10,
  algoParams: {},
  algorithmInstanceCount: 1,
  algorithmInstances: [
    {
      instanceId: 'instance-1',
      algorithmId: 'general_detection',
      analysisFps: 10,
      enabled: true,
      actualStatus: 2,
      applyState: 'applied',
      statusMessage: '',
    },
  ],
  rulesCount: 0,
  motionGateEnabled: true,
  rules: [],
  configRevision: 1,
  createdAt: 1710000000000,
  updatedAt: 1710000000000,
}

describe('CameraDeviceTile', () => {
  it('keeps the tile focused on device identity and operational metadata', () => {
    const html = renderToString(<CameraDeviceTile camera={mockCamera} />)

    expect(html).toContain('东门周界枪机')
    expect(html).toContain('1号大门主通道')
    expect(html).toContain('4K')
    expect(html).toContain('3840')
    expect(html).toContain('2160')
    expect(html).toContain('FPS')
    expect(html).toContain('Inactive')
    expect(html).not.toContain('#cam_001')
    expect(html).not.toContain('RTSP')
    expect(html).not.toContain('HEVC')
  })

  it('uses a neutral location placeholder without inventing a physical location', () => {
    const camWithoutRemark: Camera = { ...mockCamera, remark: '' }
    const html = renderToString(<CameraDeviceTile camera={camWithoutRemark} />)

    expect(html).toContain('—')
    expect(html).not.toContain('未标定物理位置')
  })

  it('expresses offline state on the camera SVG itself', () => {
    const offlineCam: Camera = { ...mockCamera, lastProbeStatus: 'failed' }
    const html = renderToString(<CameraDeviceTile camera={offlineCam} />)

    expect(html).toContain('camera-svg-offline')
    expect(html).toContain('data-camera-status="offline"')
  })

  it('renders the AI active state from the task runtime summary', () => {
    const html = renderToString(<CameraDeviceTile camera={mockCamera} task={activeTask} />)

    expect(html).toContain('AI Active')
    expect(html).toContain('camera-svg-scan')
  })

  it('renders the top-right more actions trigger and detail affordance', () => {
    const html = renderToString(
      <CameraDeviceTile
        camera={mockCamera}
        onManualProbe={() => {}}
        onEdit={() => {}}
        onDelete={() => {}}
        onClick={() => {}}
      />,
    )

    expect(html).toContain('更多操作')
    expect(html).toContain('详情')
  })

  it('renders dropdown action items when menu is open', () => {
    const html = renderToString(
      <CameraDeviceTile
        camera={mockCamera}
        defaultMenuOpen
        onManualProbe={() => {}}
        onEdit={() => {}}
        onDelete={() => {}}
        onClick={() => {}}
      />,
    )

    expect(html).toContain('查看设备详情')
    expect(html).toContain('即时探活')
    expect(html).toContain('编辑设备参数')
    expect(html).toContain('复制主流地址')
    expect(html).toContain('移除设备')
  })

  it('renders the three supported camera illustrations through explicit type selection', () => {
    const bulletHtml = renderToString(<CameraDeviceTile camera={mockCamera} cameraType="bullet" />)
    const domeHtml = renderToString(<CameraDeviceTile camera={mockCamera} cameraType="dome" />)
    const ptzHtml = renderToString(<CameraDeviceTile camera={mockCamera} cameraType="ptz" />)

    expect(bulletHtml).toContain('筒形枪机 illustration')
    expect(domeHtml).toContain('半球摄像机 illustration')
    expect(ptzHtml).toContain('云台球机 illustration')
  })

  it('does not assign role="button" to article avoiding nested interactive element ARIA violations', () => {
    const html = renderToString(
      <CameraDeviceTile camera={mockCamera} onClick={() => {}} onEdit={() => {}} />,
    )
    expect(html).not.toContain('role="button"')
    expect(html).toContain('tabindex="0"')
  })

  it('uses shared getCameraTypeLabel helper for uniform localized strings', () => {
    const fakeT = (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue || key
    expect(getCameraTypeLabel('dome', fakeT)).toBe('半球摄像机')
    expect(getCameraTypeLabel('ptz', fakeT)).toBe('云台球机')
    expect(getCameraTypeLabel('bullet', fakeT)).toBe('筒形枪机')
  })
})
