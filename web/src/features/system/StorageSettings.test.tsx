// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { systemApi } from '@/lib/system-api'
import type { SnapshotSystemConfig, StorageStatus } from '@/types/system'
import { StorageSettings } from './StorageSettings'
import type { StorageDraft } from './storageDraft'

// 模拟 react-i18next：返回 defaultValue（缺省时退化为 key），断言文案键而非语言字面量
vi.mock('react-i18next', () => ({
  initReactI18next: { type: '3rdParty', init: () => undefined },
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue || key,
    i18n: { language: 'zh-CN' },
  }),
}))

// 保存链路只断言请求体，不触达真实网络
vi.mock('@/lib/system-api', () => ({
  systemApi: {
    getStorageStatus: vi.fn(),
    getStorageConfig: vi.fn(),
    updateStorageConfig: vi.fn(),
    triggerCleanup: vi.fn(),
    getSnapshotConfig: vi.fn(),
    updateSnapshotConfig: vi.fn(),
  },
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const STATUS: StorageStatus = {
  totalGb: 100,
  usedGb: 40,
  availableGb: 60,
  usagePercent: 40,
  healthLevel: 'normal',
  alarmCount: 1,
  alarmSizeMb: 1,
  recognitionCount: 1,
  recognitionSizeMb: 1,
  captureCount: 1,
  captureSizeMb: 1,
}

const SNAPSHOT: SnapshotSystemConfig = {
  mainStreamPanoramicQuality: 90,
  mainStreamCropQuality: 95,
  subStreamPanoramicQuality: 80,
  subStreamCropQuality: 85,
  cropPaddingRatio: 0.1,
}

function makeConfig(overrides: Partial<StorageDraft> = {}): StorageDraft {
  return {
    alarmRetentionDays: 30,
    alarmQuotaMb: 0,
    recognitionRetentionDays: 14,
    recognitionQuotaMb: 0,
    captureRetentionDays: 7,
    captureQuotaMb: 0,
    recordingRetentionDays: 7,
    overwriteMode: 'overwrite',
    autoCleanupEnabled: true,
    minFreeRatio: 0.15,
    targetFreeRatio: 0.25,
    emergencyFreeRatio: 0.08,
    criticalFreeRatio: 0.05,
    batchDeleteSize: 100,
    ...overrides,
  }
}

/** 渲染并展开「高级设置」，数值控件此时全部可见 */
async function renderPage(config: StorageDraft = makeConfig()): Promise<void> {
  vi.mocked(systemApi.getStorageStatus).mockResolvedValue(STATUS)
  vi.mocked(systemApi.getStorageConfig).mockResolvedValue(config)
  vi.mocked(systemApi.getSnapshotConfig).mockResolvedValue(SNAPSHOT)
  vi.mocked(systemApi.updateStorageConfig).mockResolvedValue(config)
  render(<StorageSettings />)
  fireEvent.click(await screen.findByRole('button', { name: '高级设置' }))
}

function getInput(label: string): HTMLInputElement {
  return screen.getByLabelText(label) as HTMLInputElement
}

/** 页面里图片编码区也有一个「保存」，取保留策略区块那一个（DOM 顺序在前） */
function storageSaveButton(): HTMLButtonElement {
  return screen.getAllByRole('button', { name: '保存' })[0] as HTMLButtonElement
}

/** 触发一次非数值改动，让保存可用（数值字段本身与不变的服务端值相等时 isDirty 为 false） */
function makeDirty(): void {
  fireEvent.click(screen.getByRole('switch'))
  expect(storageSaveButton().disabled).toBe(false)
}

async function clickSaveAndGetPayload(): Promise<StorageDraft> {
  fireEvent.click(storageSaveButton())
  await waitFor(() => expect(vi.mocked(systemApi.updateStorageConfig)).toHaveBeenCalledTimes(1))
  const payload = vi.mocked(systemApi.updateStorageConfig).mock.calls[0]?.[0]
  if (!payload) throw new Error('保存请求缺少请求体')
  return payload
}

const DAYS_LABEL = '告警图 保留天数'
const QUOTA_LABEL = '告警图 配额(MB)'
const MIN_FREE_LABEL = '触发清理水位(%)'
const BATCH_LABEL = '单批删除数'

describe('StorageSettings 数值输入形态', () => {
  it('保留天数与高级设置均走共享 NumericField（type=text + inputMode）', async () => {
    await renderPage()
    const days = getInput(DAYS_LABEL)
    const batch = getInput(BATCH_LABEL)

    // 旧实现是 type="number"，浏览器会对中间态做 `value` 归一化
    expect(days.getAttribute('type')).toBe('text')
    expect(days.getAttribute('inputMode')).toBe('numeric')
    expect(batch.getAttribute('type')).toBe('text')
    expect(batch.getAttribute('inputMode')).toBe('numeric')
  })

  it('表格控件有唯一可访问名称（行类型 + 列头）', async () => {
    await renderPage()
    expect(getInput(DAYS_LABEL).value).toBe('30')
    expect(getInput(QUOTA_LABEL).value).toBe('0')
    expect(getInput('识别图 保留天数').value).toBe('14')
    expect(getInput('抓拍图 保留天数').value).toBe('7')
  })
})

describe('StorageSettings 编辑期草稿', () => {
  it('编辑期可清空并重新输入完整数值，中途不被强制回写', async () => {
    // 该用例锁定编辑期契约（旧实现本身即可满足），R8 的回归断言在下方「中间态不写入模型」
    // 与「失焦收敛」：旧 NumericInput 输入期即 live commit、失焦不做 clamp。
    await renderPage()
    const days = getInput(DAYS_LABEL)

    fireEvent.change(days, { target: { value: '' } })
    expect(days.value).toBe('')

    fireEvent.change(days, { target: { value: '1' } })
    expect(days.value).toBe('1')
    fireEvent.change(days, { target: { value: '12' } })
    expect(days.value).toBe('12')

    fireEvent.blur(days)
    expect(days.value).toBe('12')
  })

  it('中间态不写入模型：可解析但越界的草稿不解锁保存，失焦后才收敛派发', async () => {
    // 回归（PRD R8「输入期即 live commit」）：旧实现在 change 里直接 onChange(parsed)，
    // 输入 400 会立刻写进草稿并解锁保存；以下断言在迁移前为红。
    await renderPage()
    const days = getInput(DAYS_LABEL)

    fireEvent.change(days, { target: { value: '400' } })
    expect(days.value).toBe('400')
    expect(storageSaveButton().disabled).toBe(true)

    fireEvent.blur(days)
    expect(days.value).toBe('365')
    expect(storageSaveButton().disabled).toBe(false)
  })

  it('不可解析的键盘中间态不写入模型', async () => {
    await renderPage()
    const days = getInput(DAYS_LABEL)

    fireEvent.change(days, { target: { value: '-' } })
    // dirty 由失焦后的派发驱动；草稿本身不入模型，保存按钮保持禁用
    expect(storageSaveButton().disabled).toBe(true)

    fireEvent.blur(days)
    expect(days.value).toBe('30')
  })
})

describe('StorageSettings 失焦收敛', () => {
  it('保留天数超界收敛到 365，且作为兜底随保存发送', async () => {
    await renderPage()
    const days = getInput(DAYS_LABEL)

    fireEvent.change(days, { target: { value: '400' } })
    fireEvent.blur(days)
    expect(days.value).toBe('365')

    const payload = await clickSaveAndGetPayload()
    expect(payload.alarmRetentionDays).toBe(365)
  })

  it('水位输入 150 失焦收敛为 100，并写成 1 的比值', async () => {
    await renderPage()
    const minFree = getInput(MIN_FREE_LABEL)
    expect(minFree.value).toBe('15')

    fireEvent.change(minFree, { target: { value: '150' } })
    fireEvent.blur(minFree)
    expect(minFree.value).toBe('100')

    const payload = await clickSaveAndGetPayload()
    expect(payload.minFreeRatio).toBe(1)
  })

  it('单批删除数输入 0 失焦收敛为 10（旧实现区间被写死 0–100，收不到 10）', async () => {
    await renderPage()
    const batch = getInput(BATCH_LABEL)

    fireEvent.change(batch, { target: { value: '0' } })
    fireEvent.blur(batch)
    expect(batch.value).toBe('10')

    const payload = await clickSaveAndGetPayload()
    expect(payload.batchDeleteSize).toBe(10)
  })

  it('必填字段清空失焦回退上次合法值，不派发也不产生请求', async () => {
    await renderPage()
    const days = getInput(DAYS_LABEL)

    fireEvent.change(days, { target: { value: '' } })
    fireEvent.blur(days)

    expect(days.value).toBe('30')
    expect(storageSaveButton().disabled).toBe(true)
  })
})

describe('StorageSettings 配额 0=不限 语义', () => {
  it('配额 0 保持 0，不被折叠成 null 或收敛成 1', async () => {
    await renderPage()
    const quota = getInput(QUOTA_LABEL)
    expect(quota.value).toBe('0')

    fireEvent.change(quota, { target: { value: '0' } })
    fireEvent.blur(quota)
    expect(quota.value).toBe('0')

    makeDirty()
    const payload = await clickSaveAndGetPayload()
    expect(payload.alarmQuotaMb).toBe(0)
  })

  it('配额清空失焦回退上次合法值（必填语义），不会静默变成 0=不限', async () => {
    await renderPage(makeConfig({ alarmQuotaMb: 500 }))
    const quota = getInput(QUOTA_LABEL)

    fireEvent.change(quota, { target: { value: '' } })
    fireEvent.blur(quota)

    expect(quota.value).toBe('500')
  })
})

describe('StorageSettings 保存兜底', () => {
  it('未经过输入框收敛的越界取值也会被收敛后再发送', async () => {
    // 服务端可能带着历史/越界取值，用户不改这些字段直接保存时仍需 clamp
    await renderPage(
      makeConfig({
        alarmRetentionDays: 400,
        minFreeRatio: 1.5,
        batchDeleteSize: 2,
      }),
    )
    expect(getInput(DAYS_LABEL).value).toBe('400')

    // 数值字段本身未改动（草稿与服务端一致），用一次非数值切换让保存可用
    makeDirty()
    const payload = await clickSaveAndGetPayload()
    expect(payload.alarmRetentionDays).toBe(365)
    expect(payload.minFreeRatio).toBe(1)
    expect(payload.batchDeleteSize).toBe(10)
  })

  it('事件录像保留天数随保存原样带回，不被丢弃也不被改写', async () => {
    await renderPage(makeConfig({ recordingRetentionDays: 30 }))
    makeDirty()
    const payload = await clickSaveAndGetPayload()
    expect(payload.recordingRetentionDays).toBe(30)
  })
})
