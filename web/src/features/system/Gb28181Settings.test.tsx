// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { gb28181Api } from '@/lib/api'
import type { Gb28181ConfigResponse } from '@/types'
import { Gb28181Settings } from './Gb28181Settings'

// 模拟 react-i18next：返回 defaultValue（缺省时退化为 key），断言文案键而非语言字面量
vi.mock('react-i18next', () => ({
  initReactI18next: { type: '3rdParty', init: () => undefined },
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue || key,
    i18n: { language: 'zh-CN' },
  }),
}))

// 保存链路只断言请求体，不触达真实网络
vi.mock('@/lib/api', () => ({
  gb28181Api: { getConfig: vi.fn(), updateConfig: vi.fn() },
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const SIP_PORT_LABEL = 'SIP 监听端口 (Port)'
const HEARTBEAT_LABEL = '心跳超时判定时长 (秒)'

function makeResponse(
  overrides: Partial<Gb28181ConfigResponse['config']> = {},
): Gb28181ConfigResponse {
  return {
    config: {
      sipId: '34020000002000000001',
      sipDomain: '3402000000',
      sipPort: 5060,
      sipPassword: 'heimdall',
      rtpPortRangeStart: 30000,
      rtpPortRangeEnd: 30500,
      autoCatalogSync: true,
      heartbeatTimeoutSec: 180,
      updatedAtMs: 1_700_000_000_000,
      ...overrides,
    },
    health: {
      running: true,
      sipPort: 5060,
      transport: 'UDP',
      onlineDevicesCount: 0,
      totalDevicesCount: 0,
      activeStreamsCount: 0,
    },
  }
}

async function renderSettings(
  overrides: Partial<Gb28181ConfigResponse['config']> = {},
): Promise<void> {
  vi.mocked(gb28181Api.getConfig).mockResolvedValue(makeResponse(overrides))
  vi.mocked(gb28181Api.updateConfig).mockResolvedValue(makeResponse(overrides).config)
  render(<Gb28181Settings />)
  await screen.findByLabelText(SIP_PORT_LABEL)
}

function getInput(label: string): HTMLInputElement {
  return screen.getByLabelText(label) as HTMLInputElement
}

describe('Gb28181Settings 数值输入', () => {
  it('编辑期可清空并重新输入完整数值，清空不再被折成 0', async () => {
    // 回归（PRD R6）：旧实现 value 直绑 number + Number(e.target.value)，
    // 清空即变 '0'，用户无法重输。以下断言在迁移前为红。
    await renderSettings()
    const input = getInput(SIP_PORT_LABEL)

    expect(input.value).toBe('5060')

    fireEvent.change(input, { target: { value: '' } })
    expect(input.value).toBe('')

    fireEvent.change(input, { target: { value: '6060' } })
    expect(input.value).toBe('6060')

    fireEvent.blur(input)
    expect(input.value).toBe('6060')
  })

  it('必填端口清空后失焦回退上次合法值，且不产生保存请求', async () => {
    await renderSettings()
    const input = getInput(SIP_PORT_LABEL)

    fireEvent.change(input, { target: { value: '' } })
    fireEvent.blur(input)

    expect(input.value).toBe('5060')
    expect(vi.mocked(gb28181Api.updateConfig)).not.toHaveBeenCalled()
  })

  it('失焦把超界端口收敛到 65535 并显示边界值', async () => {
    await renderSettings()
    const input = getInput(SIP_PORT_LABEL)

    fireEvent.change(input, { target: { value: '70000' } })
    fireEvent.blur(input)

    expect(input.value).toBe('65535')
  })

  it('中间态不写入模型：未收敛的草稿直接保存时模型仍是上次合法值', async () => {
    await renderSettings()
    const input = getInput(SIP_PORT_LABEL)

    // 刻意选 `-` 而不是超界值：真实浏览器点击保存会先触发 blur（把超界值收敛后派发），
    // 而 jsdom 不会，两者预期不同；`-` 在两边走同一条「不可解析→回退」路径，断言跨环境成立。
    fireEvent.change(input, { target: { value: '-' } })
    fireEvent.click(screen.getByRole('button', { name: 'actions.save' }))

    await waitFor(() => expect(vi.mocked(gb28181Api.updateConfig)).toHaveBeenCalledTimes(1))
    expect(vi.mocked(gb28181Api.updateConfig).mock.calls[0]?.[0]).toMatchObject({
      sipPort: 5060,
    })
  })

  it('保存请求携带收敛后的数值（含超界兜底）', async () => {
    await renderSettings()

    const sipPort = getInput(SIP_PORT_LABEL)
    fireEvent.change(sipPort, { target: { value: '70000' } })
    fireEvent.blur(sipPort)

    const heartbeat = getInput(HEARTBEAT_LABEL)
    fireEvent.change(heartbeat, { target: { value: '200000' } })
    fireEvent.blur(heartbeat)
    expect(heartbeat.value).toBe('86400')

    fireEvent.click(screen.getByRole('button', { name: 'actions.save' }))

    await waitFor(() => expect(vi.mocked(gb28181Api.updateConfig)).toHaveBeenCalledTimes(1))
    expect(vi.mocked(gb28181Api.updateConfig).mock.calls[0]?.[0]).toMatchObject({
      sipPort: 65535,
      rtpPortRangeStart: 30000,
      rtpPortRangeEnd: 30500,
      heartbeatTimeoutSec: 86400,
    })
  })

  it('保存路径是独立兜底：未经过输入框的越界取值也会被收敛后再发送', async () => {
    // 服务端可能带着历史/软上界外的取值（心跳上界是操作意义上界，非 wire 硬约束），
    // 直接点保存不得把未收敛值透传出去。
    await renderSettings({ sipPort: 0, heartbeatTimeoutSec: 200000 })
    expect(getInput(SIP_PORT_LABEL).value).toBe('0')

    fireEvent.click(screen.getByRole('button', { name: 'actions.save' }))

    await waitFor(() => expect(vi.mocked(gb28181Api.updateConfig)).toHaveBeenCalledTimes(1))
    expect(vi.mocked(gb28181Api.updateConfig).mock.calls[0]?.[0]).toMatchObject({
      sipPort: 1,
      heartbeatTimeoutSec: 86400,
    })
  })
})
