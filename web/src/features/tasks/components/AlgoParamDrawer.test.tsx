// @vitest-environment jsdom
import { renderToString } from 'react-dom/server'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { AlgoManifest } from '@/types'
import type { SystemOverview } from '@/types/system'
import { systemApi } from '@/lib/system-api'
import { AlgoParamDrawer } from './AlgoParamDrawer'

vi.mock('@/lib/system-api', () => ({
  systemApi: {
    getOverview: vi.fn(),
  },
}))

vi.mock('react-i18next', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-i18next')>()
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: { defaultValue?: string; percent?: string | number }) => {
        let text = opts?.defaultValue || key
        if (opts?.percent !== undefined) {
          text = text.replace('{{percent}}', String(opts.percent))
        }
        return text
      },
      i18n: { language: 'zh-CN' },
    }),
  }
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

/**
 * 阈值类参数（`similarity_threshold`）与底库检索输出同处标定置信分域 [0, 1]，
 * 控制台必须原样下发：界面设 75% 时 `params_json` 必须得到 0.75。
 */
const faceRecognitionAlgo = {
  algorithmId: 'face_recognition',
  name: 'Face Recognition',
  version: '1.0.0',
  category: 'recognition',
  configSchema: {
    type: 'object',
    properties: {
      similarity_threshold: {
        type: 'number',
        title: '识别确认阈值',
        minimum: 0,
        maximum: 1,
        default: 0.75,
      },
      detection_confidence_threshold: {
        type: 'number',
        title: '检测置信度阈值',
        minimum: 0,
        maximum: 1,
        default: 0.6,
      },
    },
  },
} as unknown as AlgoManifest

function renderDrawer(params: Record<string, unknown>): string {
  return renderToString(
    <AlgoParamDrawer
      isOpen
      algo={faceRecognitionAlgo}
      fps={10}
      onFpsChange={() => {}}
      params={params}
      onSaveParams={() => {}}
      onClose={() => {}}
    />,
  )
}

describe('AlgoParamDrawer threshold calibration-domain passthrough', () => {
  it('renders the threshold as a direct percentage of the stored score', () => {
    const html = renderDrawer({ similarity_threshold: 0.75 })
    expect(html).toContain('75.0%')
    expect(html).not.toContain('93.1%')
  })

  it('renders the full 0-100 range instead of the compressed calibration window', () => {
    const html = renderDrawer({ similarity_threshold: 0.75 })
    expect(html).toMatch(/min="0"[^>]*max="100"/)
  })

  it('round-trips stored scores without drifting through a calibration transform', () => {
    expect(renderDrawer({ similarity_threshold: 0.456 })).toContain('45.6%')
    expect(renderDrawer({ similarity_threshold: 0.8 })).toContain('80.0%')
  })

  it('leaves non-threshold numeric params untouched', () => {
    const html = renderDrawer({ detection_confidence_threshold: 0.6 })
    expect(html).toMatch(/value="0\.6"/)
  })

  it('renders affinity section with default auto spread policy in SSR', () => {
    const html = renderDrawer({})
    expect(html).toContain('Auto·Spread')
    expect(html).toContain('自动调度 (Auto)')
  })
})

describe('AlgoParamDrawer affinity interactive scheduling', () => {
  const mockMultiCoreOverview = {
    npu: {
      deviceType: 'Rockchip RKNN',
      cores: [
        { coreId: 0, utilizationPercent: 32.4, frequencyMhz: 1000, powerWatts: null },
        { coreId: 1, utilizationPercent: 48.1, frequencyMhz: 1000, powerWatts: null },
        { coreId: 2, utilizationPercent: 15.6, frequencyMhz: 1000, powerWatts: null },
      ],
      totalMemoryMb: 8192,
      usedMemoryMb: 2048,
      temperature: 46.5,
      activeSessions: 2,
      inferenceCount: 1200,
    },
  } as unknown as SystemOverview

  it('probes multi-core NPU topology and applies manual core selection', async () => {
    vi.mocked(systemApi.getOverview).mockResolvedValue(mockMultiCoreOverview)
    const handleAffinityChange = vi.fn()
    const handleSaveParams = vi.fn()
    const handleClose = vi.fn()

    render(
      <AlgoParamDrawer
        isOpen
        algo={faceRecognitionAlgo}
        fps={10}
        onFpsChange={() => {}}
        affinity={{ mode: 'auto', policy: 'spread' }}
        onAffinityChange={handleAffinityChange}
        params={{}}
        onSaveParams={handleSaveParams}
        onClose={handleClose}
      />,
    )

    // 等待多核拓扑探测完成
    await waitFor(() => {
      expect(systemApi.getOverview).toHaveBeenCalledTimes(1)
    })

    // 验证多核提示出现
    await waitFor(() => {
      expect(
        screen.getByText(
          '多核异构调度：默认 Auto 均衡策略可实现双/多核心并行推理，避免单核过载瓶颈。',
        ),
      ).toBeTruthy()
    })

    // 切换至手动切核模式
    const manualBtn = screen.getByRole('button', { name: /手动指定核心/ })
    expect(manualBtn.hasAttribute('disabled')).toBe(false)
    fireEvent.click(manualBtn)

    // 应展示物理核心选项及其利用率
    expect(screen.getByRole('button', { name: /Core 0/ })).toBeTruthy()
    expect(screen.getByRole('button', { name: /Core 1/ })).toBeTruthy()
    expect(screen.getByRole('button', { name: /Core 2/ })).toBeTruthy()
    expect(screen.getByText('32% 负载')).toBeTruthy()
    expect(screen.getByText('48% 负载')).toBeTruthy()

    // 选中 Core 2
    fireEvent.click(screen.getByRole('button', { name: /Core 2/ }))

    // 点击应用
    const applyBtn = screen.getByRole('button', { name: /应用参数/ })
    fireEvent.click(applyBtn)

    expect(handleAffinityChange).toHaveBeenCalledWith({
      mode: 'manual',
      deviceId: 'rknn-npu0',
      coreIndex: 2,
    })
    expect(handleClose).toHaveBeenCalled()
  })

  it('preserves existing custom deviceId when switching to manual mode', async () => {
    vi.mocked(systemApi.getOverview).mockResolvedValue(mockMultiCoreOverview)
    const handleAffinityChange = vi.fn()

    render(
      <AlgoParamDrawer
        isOpen
        algo={faceRecognitionAlgo}
        fps={10}
        onFpsChange={() => {}}
        affinity={{ mode: 'manual', deviceId: 'custom-npu1', coreIndex: 0 }}
        onAffinityChange={handleAffinityChange}
        params={{}}
        onSaveParams={() => {}}
        onClose={() => {}}
      />,
    )

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Core 1/ })).toBeTruthy()
    })

    // 切换至 Core 1
    fireEvent.click(screen.getByRole('button', { name: /Core 1/ }))

    // 点击应用
    const applyBtn = screen.getByRole('button', { name: /应用参数/ })
    fireEvent.click(applyBtn)

    // deviceId 应保留 'custom-npu1' 而非被硬编码洗成 'rknn-npu0'
    expect(handleAffinityChange).toHaveBeenCalledWith({
      mode: 'manual',
      deviceId: 'custom-npu1',
      coreIndex: 1,
    })
  })

  it('disables manual mode on single-core devices', async () => {
    const singleCoreOverview = {
      npu: {
        deviceType: 'Rockchip RKNN',
        cores: [{ coreId: 0, utilizationPercent: 20.0, frequencyMhz: 1000, powerWatts: null }],
        totalMemoryMb: 2048,
        usedMemoryMb: 512,
        temperature: 40.0,
        activeSessions: 1,
        inferenceCount: 100,
      },
    } as unknown as SystemOverview

    vi.mocked(systemApi.getOverview).mockResolvedValue(singleCoreOverview)

    render(
      <AlgoParamDrawer
        isOpen
        algo={faceRecognitionAlgo}
        fps={10}
        onFpsChange={() => {}}
        affinity={{ mode: 'auto', policy: 'spread' }}
        params={{}}
        onSaveParams={() => {}}
        onClose={() => {}}
      />,
    )

    await waitFor(() => {
      const manualBtn = screen.getByRole('button', { name: /手动指定核心/ })
      expect(manualBtn.hasAttribute('disabled')).toBe(true)
      expect(
        screen.getByText('当前运行环境为单核 NPU 或宿主托管模式，已由底层驱动自动优化调度。'),
      ).toBeTruthy()
    })
  })
})
