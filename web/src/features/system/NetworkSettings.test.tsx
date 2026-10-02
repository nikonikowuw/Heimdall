// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { systemApi } from '@/lib/system-api'
import type { NetworkInterface } from '@/types/system'
import { NetworkSettings } from './NetworkSettings'

// 模拟 react-i18next：返回 defaultValue（缺省时退化为 key），断言文案键而非语言字面量
vi.mock('react-i18next', () => ({
  initReactI18next: { type: '3rdParty', init: () => undefined },
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue || key,
    i18n: { language: 'zh-CN' },
  }),
}))

// 只断言请求体，不触达真实网络
vi.mock('@/lib/system-api', () => ({
  systemApi: {
    getNetworkInterfaces: vi.fn(),
    updateNetworkInterface: vi.fn(),
    getPendingNetworkChange: vi.fn(),
    confirmNetworkChange: vi.fn(),
    cancelNetworkChange: vi.fn(),
  },
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const PREFIX_LABEL = '前缀'
const METRIC_LABEL = 'Metric'

function makeIface(overrides: Partial<NetworkInterface['ipv4']> = {}): NetworkInterface {
  return {
    name: 'eth0',
    type: 'ethernet',
    state: 'up',
    carrier: true,
    speed: 1000,
    duplex: 'full',
    mac: 'AA:BB:CC:DD:EE:FF',
    manager: 'networkmanager',
    ipv4: {
      method: 'static',
      address: '192.168.1.100',
      prefix: 24,
      gateway: '192.168.1.1',
      dns: ['8.8.8.8'],
      metric: null,
      ...overrides,
    },
    capabilities: {
      canModifyIp: true,
      canSetDhcp: true,
      canSetStatic: true,
      isManagementInterface: false,
      reason: null,
    },
  }
}

async function renderEditing(iface: NetworkInterface = makeIface()): Promise<void> {
  vi.mocked(systemApi.getNetworkInterfaces).mockResolvedValue({
    interfaces: [iface],
    pendingOperation: null,
  })
  vi.mocked(systemApi.updateNetworkInterface).mockResolvedValue({
    applied: true,
    operation: null,
  })
  render(<NetworkSettings />)
  fireEvent.click(await screen.findByRole('button', { name: '编辑' }))
  await screen.findByLabelText(PREFIX_LABEL)
}

function getInput(label: string): HTMLInputElement {
  return screen.getByLabelText(label) as HTMLInputElement
}

describe('NetworkSettings 数值输入', () => {
  it('静态前缀与 Metric 走共享 NumericField 的输入形态', async () => {
    await renderEditing()
    const prefix = getInput(PREFIX_LABEL)
    const metric = getInput(METRIC_LABEL)

    // type="text" + inputMode 是 NumericField 的特征（旧实现是 type="number"），
    // 保证中间态不被浏览器 `value` 归一化丢掉
    expect(prefix.getAttribute('type')).toBe('text')
    expect(prefix.getAttribute('inputMode')).toBe('numeric')
    expect(metric.getAttribute('type')).toBe('text')
    expect(metric.getAttribute('inputMode')).toBe('numeric')
  })

  it('前缀清空后保持空白，不再被折成 0，也不写入 NaN', async () => {
    // 旧实现 `value={draft.prefix || ''}` 在清空时同样显示空白（此断言两边皆绿）；
    // R7 的真正回归点是下方的「0 → clamp(1)」——旧 `Number(v) || null` 会把 0 折成 null。
    await renderEditing()
    const prefix = getInput(PREFIX_LABEL)

    expect(prefix.value).toBe('24')

    fireEvent.change(prefix, { target: { value: '' } })
    expect(prefix.value).toBe('')

    fireEvent.blur(prefix)
    expect(prefix.value).toBe('')

    fireEvent.click(screen.getByRole('button', { name: '保存并应用' }))
    await waitFor(() =>
      expect(vi.mocked(systemApi.updateNetworkInterface)).toHaveBeenCalledTimes(1),
    )
    expect(vi.mocked(systemApi.updateNetworkInterface).mock.calls[0]?.[1].prefix).toBeUndefined()
  })

  it('前缀输入 0 失焦收敛为 1 并显示 1（修复 0 → null 折叠）', async () => {
    await renderEditing()
    const prefix = getInput(PREFIX_LABEL)

    fireEvent.change(prefix, { target: { value: '0' } })
    fireEvent.blur(prefix)

    expect(prefix.value).toBe('1')

    fireEvent.click(screen.getByRole('button', { name: '保存并应用' }))
    await waitFor(() =>
      expect(vi.mocked(systemApi.updateNetworkInterface)).toHaveBeenCalledTimes(1),
    )
    expect(vi.mocked(systemApi.updateNetworkInterface).mock.calls[0]?.[1]).toMatchObject({
      prefix: 1,
    })
  })

  it('前缀超界失焦收敛到 32，并作为最终兜底随保存发送', async () => {
    await renderEditing()
    const prefix = getInput(PREFIX_LABEL)

    fireEvent.change(prefix, { target: { value: '40' } })
    fireEvent.blur(prefix)
    expect(prefix.value).toBe('32')

    fireEvent.click(screen.getByRole('button', { name: '保存并应用' }))
    await waitFor(() =>
      expect(vi.mocked(systemApi.updateNetworkInterface)).toHaveBeenCalledTimes(1),
    )
    expect(vi.mocked(systemApi.updateNetworkInterface).mock.calls[0]?.[1]).toMatchObject({
      prefix: 32,
    })
  })

  it('保存路径是独立兜底：未经过输入框收敛的越界前缀也会被收敛后再发送', async () => {
    // 服务端可能带着历史取值；用户不改前缀直接保存时，仍需 clamp 到 1–32
    await renderEditing(makeIface({ prefix: 40 }))
    expect(getInput(PREFIX_LABEL).value).toBe('40')

    fireEvent.click(screen.getByRole('button', { name: '保存并应用' }))
    await waitFor(() =>
      expect(vi.mocked(systemApi.updateNetworkInterface)).toHaveBeenCalledTimes(1),
    )
    expect(vi.mocked(systemApi.updateNetworkInterface).mock.calls[0]?.[1]).toMatchObject({
      prefix: 32,
    })
  })

  it('Metric 可空：清空失焦保持空白且保存不带值，输入数值照常提交', async () => {
    await renderEditing()
    const metric = getInput(METRIC_LABEL)

    expect(metric.value).toBe('')

    fireEvent.change(metric, { target: { value: '100' } })
    fireEvent.blur(metric)
    expect(metric.value).toBe('100')

    fireEvent.click(screen.getByRole('button', { name: '保存并应用' }))
    await waitFor(() =>
      expect(vi.mocked(systemApi.updateNetworkInterface)).toHaveBeenCalledTimes(1),
    )
    expect(vi.mocked(systemApi.updateNetworkInterface).mock.calls[0]?.[1]).toMatchObject({
      metric: 100,
    })
  })

  it('中间态不写入模型：未失焦的不可解析草稿不影响保存载荷', async () => {
    // 旧实现在输入期即 live commit：输入 `-` 会把 draft.prefix 写成 null，
    // 保存发 undefined；新实现下模型保持上次合法值 24。
    // 选 `-` 而非超界值：真实浏览器点击保存会先 blur（超界值收敛后派发），jsdom 不会；
    // `-` 两边都走「不可解析→回退」，断言跨环境成立。
    await renderEditing()
    const prefix = getInput(PREFIX_LABEL)

    fireEvent.change(prefix, { target: { value: '-' } })
    fireEvent.click(screen.getByRole('button', { name: '保存并应用' }))

    await waitFor(() =>
      expect(vi.mocked(systemApi.updateNetworkInterface)).toHaveBeenCalledTimes(1),
    )
    expect(vi.mocked(systemApi.updateNetworkInterface).mock.calls[0]?.[1].prefix).toBe(24)
  })

  it('中间态不写入模型：编辑中取消，重新进入仍显示服务端值', async () => {
    await renderEditing()
    const prefix = getInput(PREFIX_LABEL)

    fireEvent.change(prefix, { target: { value: '999' } })
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    fireEvent.click(screen.getByRole('button', { name: '编辑' }))

    expect(getInput(PREFIX_LABEL).value).toBe('24')
    expect(vi.mocked(systemApi.updateNetworkInterface)).not.toHaveBeenCalled()
  })
})
