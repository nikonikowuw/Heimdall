import { describe, expect, it } from 'vitest'
import type { TaskAlgorithmInstanceSummaryDto, TaskSummaryDto } from '@/types'
import { buildAlgoUsage, blockingUsageEntries, usageEntriesFor } from './algoUsage'

function instance(
  overrides: Partial<TaskAlgorithmInstanceSummaryDto> = {},
): TaskAlgorithmInstanceSummaryDto {
  return {
    instanceId: 'i1',
    algorithmId: 'general_detection',
    analysisFps: 15,
    enabled: true,
    actualStatus: 1,
    applyState: 'applied',
    statusMessage: '',
    ...overrides,
  }
}

function makeTask(overrides: Partial<TaskSummaryDto> = {}): TaskSummaryDto {
  return {
    id: 1,
    cameraId: 'CAM-01',
    name: '大门通道',
    desiredEnabled: true,
    actualStatus: 1,
    rulesCount: 0,
    motionGateEnabled: false,
    rules: [],
    configRevision: 1,
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
    ...overrides,
  }
}

describe('buildAlgoUsage', () => {
  it('按算法聚合占用通道与任务名', () => {
    const usage = buildAlgoUsage([
      makeTask({
        cameraId: 'CAM-01',
        name: '大门通道',
        algorithmInstances: [instance({ enabled: true })],
      }),
      makeTask({
        id: 2,
        cameraId: 'CAM-02',
        name: '仓库通道',
        algorithmInstances: [instance({ instanceId: 'i2', enabled: false })],
      }),
    ])

    expect(usageEntriesFor(usage, 'general_detection')).toEqual([
      { cameraId: 'CAM-01', taskName: '大门通道', enabled: true },
      { cameraId: 'CAM-02', taskName: '仓库通道', enabled: false },
    ])
  })

  it('同一通道上的多实例只记一条，占用口径是通道数而不是实例数', () => {
    const usage = buildAlgoUsage([
      makeTask({
        algorithmInstances: [instance(), instance({ instanceId: 'i2' })],
      }),
    ])

    expect(usageEntriesFor(usage, 'general_detection')).toHaveLength(1)
  })

  it('同一任务的多个不同算法分别归集', () => {
    const usage = buildAlgoUsage([
      makeTask({
        algorithmInstances: [
          instance(),
          instance({ instanceId: 'i2', algorithmId: 'face_recognition' }),
        ],
      }),
    ])

    expect(usage.size).toBe(2)
    expect(usageEntriesFor(usage, 'face_recognition')).toHaveLength(1)
  })

  it('容忍缺失实例列表与空算法 ID', () => {
    const usage = buildAlgoUsage([
      makeTask({ cameraId: 'CAM-NO-INST' }),
      makeTask({
        cameraId: 'CAM-TRIMMED',
        algorithmInstances: [instance({ algorithmId: '  ' })],
      }),
    ])

    expect(usage.size).toBe(0)
    expect(usageEntriesFor(usage, 'general_detection')).toEqual([])
  })

  it('占用通道按通道 ID 稳定排序', () => {
    const usage = buildAlgoUsage([
      makeTask({ cameraId: 'CAM-Z', algorithmInstances: [instance({ algorithmId: 'algo' })] }),
      makeTask({
        id: 2,
        cameraId: 'CAM-A',
        algorithmInstances: [instance({ instanceId: 'i2', algorithmId: 'algo' })],
      }),
    ])

    expect(usageEntriesFor(usage, 'algo').map((entry) => entry.cameraId)).toEqual([
      'CAM-A',
      'CAM-Z',
    ])
  })

  it('空任务清单返回空映射', () => {
    expect(buildAlgoUsage([]).size).toBe(0)
  })
})

describe('blockingUsageEntries', () => {
  it('只保留启用中的占用，与后端 count_active_instances 口径一致', () => {
    const entries = [
      { cameraId: 'CAM-01', taskName: '大门', enabled: true },
      { cameraId: 'CAM-02', taskName: '仓库', enabled: false },
    ]
    expect(blockingUsageEntries(entries).map((entry) => entry.cameraId)).toEqual(['CAM-01'])
  })

  it('全部停用时不会阻断卸载', () => {
    expect(
      blockingUsageEntries([{ cameraId: 'CAM-01', taskName: '大门', enabled: false }]),
    ).toEqual([])
  })
})
