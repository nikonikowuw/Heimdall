// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { cameraApi } from '@/lib/api'
import type { Camera } from '@/types'
import { RecordingConfigCard } from './RecordingConfigCard'

// 模拟 react-i18next：返回 key 本身（或 defaultValue），断言文案键而非语言相关字面量
vi.mock('react-i18next', () => ({
  initReactI18next: { type: '3rdParty', init: () => undefined },
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue || key,
    i18n: { language: 'zh-CN' },
  }),
}))

// 保存链路只断言请求体，不触达真实网络
vi.mock('@/lib/api', () => ({
  cameraApi: { update: vi.fn() },
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function makeCamera(overrides: Partial<Camera> = {}): Camera {
  return {
    id: 1,
    cameraId: 'cam_001',
    name: '东门周界枪机',
    protocol: 'rtsp',
    rtspUrl: 'rtsp://192.168.1.100:554/live/main',
    subRtspUrl: '',
    streamMode: 'auto',
    remark: '',
    lastProbeStatus: 'healthy',
    lastProbeAt: null,
    lastCodec: 'h264',
    lastWidth: 1920,
    lastHeight: 1080,
    lastFps: 25,
    recordingConfig: {
      enabled: true,
      preCaptureSeconds: 10,
      postCaptureSeconds: 10,
      maxFileSeconds: 300,
      retentionDays: 7,
    },
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
    ...overrides,
  }
}

function renderCard(camera: Camera = makeCamera()): { onSaved: (next: Camera) => void } {
  const onSaved = vi.fn()
  render(<RecordingConfigCard camera={camera} onSaved={onSaved} />)
  return { onSaved }
}

describe('RecordingConfigCard 数值输入', () => {
  it('编辑期可清空并重新输入完整数值，中途不被强制回写', () => {
    // 回归：旧实现把 value 直绑 number 并用 parseInt 拦中间态，清空后
    // React 把输入框弹回 '10'，用户无法重输。以下断言在迁移前均为红。
    renderCard()
    const input = screen.getByLabelText(/事件前录制/) as HTMLInputElement

    expect(input.value).toBe('10')

    fireEvent.change(input, { target: { value: '' } })
    expect(input.value).toBe('')

    fireEvent.change(input, { target: { value: '2' } })
    expect(input.value).toBe('2')
  })

  it('中间态不写入模型：未失焦时保存按钮保持禁用', () => {
    renderCard()
    const input = screen.getByLabelText(/事件前录制/) as HTMLInputElement
    const save = screen.getByRole('button', { name: '保存参数' }) as HTMLButtonElement

    fireEvent.change(input, { target: { value: '25' } })
    expect(save.disabled).toBe(true)

    fireEvent.blur(input)
    expect(input.value).toBe('25')
    expect(save.disabled).toBe(false)
  })

  it('失焦把超界值收敛到 RECORDING_LIMITS 边界并显示边界值', () => {
    renderCard()
    const input = screen.getByLabelText(/单文件上限/) as HTMLInputElement

    fireEvent.change(input, { target: { value: '99999' } })
    fireEvent.blur(input)

    expect(input.value).toBe('3600')
  })

  it('保存请求携带收敛后的参数', async () => {
    vi.mocked(cameraApi.update).mockResolvedValue(makeCamera())
    const { onSaved } = renderCard()

    const input = screen.getByLabelText(/单文件上限/) as HTMLInputElement
    fireEvent.change(input, { target: { value: '99999' } })
    fireEvent.blur(input)

    fireEvent.click(screen.getByRole('button', { name: '保存参数' }))

    await waitFor(() => expect(vi.mocked(cameraApi.update)).toHaveBeenCalledTimes(1))
    expect(vi.mocked(cameraApi.update).mock.calls[0]).toEqual([
      'cam_001',
      {
        recordingConfig: {
          enabled: true,
          preCaptureSeconds: 10,
          postCaptureSeconds: 10,
          maxFileSeconds: 3600,
          retentionDays: 7,
        },
      },
    ])
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(makeCamera()))
  })
})
