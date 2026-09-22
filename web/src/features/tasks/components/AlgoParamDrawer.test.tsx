import { renderToString } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { AlgoManifest } from '@/types'
import { AlgoParamDrawer } from './AlgoParamDrawer'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue || key,
    i18n: { language: 'zh-CN' },
  }),
}))

/**
 * 阈值类参数（`similarity_threshold`）与底库检索输出同处标定置信分域 [0, 1]，
 * 控制台必须原样下发：界面设 75% 时 `params_json` 必须得到 0.75。
 *
 * 历史缺陷：此处曾把百分比经「原始余弦标定曲线」反解，导致 75% 被写成 0.456，
 * 而宿主 `capture_service` 按标定域把 0.456 当作 45.6% 消费，
 * 确认门槛静默丢失近 30 个点，低分比对面孔被大量误放行入库。
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
    // 标定分 0.75 -> 显示 75.0%；若误用标定曲线会显示 93.1%
    const html = renderDrawer({ similarity_threshold: 0.75 })
    expect(html).toContain('75.0%')
    expect(html).not.toContain('93.1%')
  })

  it('renders the full 0-100 range instead of the compressed calibration window', () => {
    // 旧实现把 [0,1] 经标定曲线压成 50%~100%，滑块下限被错误截断
    const html = renderDrawer({ similarity_threshold: 0.75 })
    expect(html).toMatch(/min="0"[^>]*max="100"/)
  })

  it('round-trips stored scores without drifting through a calibration transform', () => {
    // 直通语义：存 0.456 就显示 45.6%，不再出现 68.0% 之类的曲线映射结果
    expect(renderDrawer({ similarity_threshold: 0.456 })).toContain('45.6%')
    expect(renderDrawer({ similarity_threshold: 0.8 })).toContain('80.0%')
  })

  it('leaves non-threshold numeric params untouched', () => {
    // detection_confidence_threshold 不是标定域阈值，必须保持原样显示
    const html = renderDrawer({ detection_confidence_threshold: 0.6 })
    expect(html).toMatch(/value="0\.6"/)
  })
})
