import { describe, expect, it } from 'vitest'
import type { Camera, TaskConfigDto } from '@/types'
import {
  ALGORITHM_FILTER_ALL,
  ARM_STATUS_FILTERS,
  countByArmStatus,
  deriveAlgorithmOptions,
  filterTasks,
  hasActiveTaskFilters,
  matchesTaskAlgorithm,
  matchesTaskArmStatus,
  matchesTaskQuery,
  normalizeTaskQuery,
  type TaskFilterEntry,
} from './taskFilter'

function makeCamera(overrides: Partial<Camera> = {}): Camera {
  return {
    id: 1,
    cameraId: 'cam-001',
    name: '东门周界枪机',
    protocol: 'rtsp',
    rtspUrl: 'rtsp://192.168.1.100:554/live/main',
    subRtspUrl: 'rtsp://192.168.1.100:554/live/sub',
    streamMode: 'auto',
    remark: '',
    lastProbeStatus: 'healthy',
    lastCodec: 'h264',
    lastWidth: 1920,
    lastHeight: 1080,
    lastFps: 25,
    recordingConfig: null,
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
    ...overrides,
  }
}

function makeConfig(overrides: Partial<TaskConfigDto> = {}): TaskConfigDto {
  return {
    cameraId: 'cam-001',
    name: '东门周界枪机',
    desiredEnabled: false,
    rules: [],
    ...overrides,
  }
}

function entry(camera: Camera, config: TaskConfigDto): TaskFilterEntry {
  return { camera, config }
}

const eastCamera = makeCamera({ cameraId: 'cam-001', name: '东门周界枪机' })
const westCamera = makeCamera({ id: 2, cameraId: 'cam-002', name: '西门球机' })

describe('normalizeTaskQuery', () => {
  it('trims and folds case', () => {
    expect(normalizeTaskQuery('  Gate-East 01  ')).toBe('gate-east 01')
  })

  it('collapses a whitespace-only query to an empty string', () => {
    expect(normalizeTaskQuery('   ')).toBe('')
  })
})

describe('matchesTaskQuery', () => {
  it('matches the task name', () => {
    const config = makeConfig({ name: '库房正门周界' })
    expect(matchesTaskQuery(config, eastCamera, '库房')).toBe(true)
    expect(matchesTaskQuery(config, eastCamera, '库房x')).toBe(false)
  })

  it('matches the camera name', () => {
    const config = makeConfig({ name: 'Warehouse Guard' })
    expect(matchesTaskQuery(config, eastCamera, '东门')).toBe(true)
  })

  it('matches the camera ID case-insensitively', () => {
    const config = makeConfig({ name: 'Warehouse Guard' })
    expect(matchesTaskQuery(config, eastCamera, 'CAM-001')).toBe(true)
  })

  it('treats an empty or whitespace-only query as no filter', () => {
    const config = makeConfig({ name: 'Whatever' })
    expect(matchesTaskQuery(config, eastCamera, '')).toBe(true)
    expect(matchesTaskQuery(config, eastCamera, '   ')).toBe(true)
  })

  it('falls back to the task name when the camera record is missing', () => {
    const config = makeConfig({ name: '孤儿任务' })
    expect(matchesTaskQuery(config, undefined, '孤儿')).toBe(true)
    expect(matchesTaskQuery(config, undefined, '东门')).toBe(false)
  })

  it('produces the same result with a pre-normalized query', () => {
    const config = makeConfig({ name: 'Front Gate' })
    const preNormalized = normalizeTaskQuery('  FRONT ')
    expect(matchesTaskQuery(config, eastCamera, '', preNormalized)).toBe(true)
    expect(matchesTaskQuery(config, eastCamera, '  FRONT ')).toBe(true)
  })
})

describe('matchesTaskArmStatus', () => {
  it('passes both statuses under the all option', () => {
    expect(matchesTaskArmStatus(makeConfig({ desiredEnabled: true }), 'all')).toBe(true)
    expect(matchesTaskArmStatus(makeConfig({ desiredEnabled: false }), 'all')).toBe(true)
  })

  it('buckets by desiredEnabled', () => {
    expect(matchesTaskArmStatus(makeConfig({ desiredEnabled: true }), 'armed')).toBe(true)
    expect(matchesTaskArmStatus(makeConfig({ desiredEnabled: true }), 'disarmed')).toBe(false)
    expect(matchesTaskArmStatus(makeConfig({ desiredEnabled: false }), 'disarmed')).toBe(true)
    expect(matchesTaskArmStatus(makeConfig({ desiredEnabled: false }), 'armed')).toBe(false)
  })
})

describe('hasActiveTaskFilters', () => {
  const none = { query: '', armStatus: 'all' as const, algorithmId: ALGORITHM_FILTER_ALL }

  it('reports no filter for the default state', () => {
    expect(hasActiveTaskFilters(none)).toBe(false)
  })

  it('treats a whitespace-only query as no filter', () => {
    expect(hasActiveTaskFilters({ ...none, query: '   ' })).toBe(false)
    expect(hasActiveTaskFilters({ ...none, query: ' 东门 ' })).toBe(true)
  })

  it('reports each non-default dimension on its own', () => {
    expect(hasActiveTaskFilters({ ...none, armStatus: 'armed' })).toBe(true)
    expect(hasActiveTaskFilters({ ...none, algorithmId: 'fire_detections' })).toBe(true)
  })

  it('stays consistent with filterTasks on whether anything was applied', () => {
    const entries = [entry(eastCamera, makeConfig({ desiredEnabled: true }))]
    // 短路条件与按钮可见性必须同源：无筛选时返回原引用，与 false 一一对应
    expect(filterTasks(entries, { ...none, query: '   ' })).toBe(entries)
    expect(filterTasks(entries, { ...none, armStatus: 'armed' })).not.toBe(entries)
  })
})

describe('matchesTaskAlgorithm', () => {
  const multi = makeConfig({
    algorithmInstances: [
      { algorithmId: 'general_detection', analysisFps: 10, enabled: true, actualStatus: 2 },
      { algorithmId: 'fire_detections', analysisFps: 5, enabled: true, actualStatus: 2 },
    ],
  })

  it('passes every task under the all option', () => {
    expect(matchesTaskAlgorithm(multi, ALGORITHM_FILTER_ALL)).toBe(true)
  })

  it('matches a task through any of its instances, not just the primary one', () => {
    expect(matchesTaskAlgorithm(multi, 'general_detection')).toBe(true)
    expect(matchesTaskAlgorithm(multi, 'fire_detections')).toBe(true)
  })

  it('rejects a task that does not mount the selected algorithm', () => {
    expect(matchesTaskAlgorithm(multi, 'face_recognition')).toBe(false)
  })

  it('does not match any concrete algorithm when instances are missing', () => {
    expect(matchesTaskAlgorithm(makeConfig(), 'fire_detections')).toBe(false)
    expect(matchesTaskAlgorithm(makeConfig(), ALGORITHM_FILTER_ALL)).toBe(true)
  })
})

describe('filterTasks', () => {
  const entries: TaskFilterEntry[] = [
    entry(eastCamera, makeConfig({ name: '东门周界', desiredEnabled: true })),
    entry(westCamera, makeConfig({ name: '西门周界', desiredEnabled: false })),
    entry(
      makeCamera({ id: 3, cameraId: 'cam-003', name: '库房枪机' }),
      makeConfig({
        cameraId: 'cam-003',
        name: '库房火焰',
        desiredEnabled: true,
        algorithmInstances: [{ algorithmId: 'fire_detections', analysisFps: 5 }],
      }),
    ),
  ]

  it('returns the same array reference when no filter is active', () => {
    const result = filterTasks(entries, {
      query: '',
      armStatus: 'all',
      algorithmId: ALGORITHM_FILTER_ALL,
    })
    expect(result).toBe(entries)
  })

  it('returns the same array reference for a whitespace-only query', () => {
    const result = filterTasks(entries, {
      query: '   ',
      armStatus: 'all',
      algorithmId: ALGORITHM_FILTER_ALL,
    })
    expect(result).toBe(entries)
  })

  it('filters by arm status alone', () => {
    const armed = filterTasks(entries, {
      query: '',
      armStatus: 'armed',
      algorithmId: ALGORITHM_FILTER_ALL,
    })
    expect(armed.map((item) => item.camera.cameraId)).toEqual(['cam-001', 'cam-003'])

    const disarmed = filterTasks(entries, {
      query: '',
      armStatus: 'disarmed',
      algorithmId: ALGORITHM_FILTER_ALL,
    })
    expect(disarmed.map((item) => item.camera.cameraId)).toEqual(['cam-002'])
  })

  it('filters by algorithm alone through any matching instance', () => {
    const fireOnly = filterTasks(entries, {
      query: '',
      armStatus: 'all',
      algorithmId: 'fire_detections',
    })
    expect(fireOnly.map((item) => item.camera.cameraId)).toEqual(['cam-003'])
  })

  it('combines all three dimensions with AND semantics', () => {
    const result = filterTasks(entries, {
      query: '库房',
      armStatus: 'armed',
      algorithmId: 'fire_detections',
    })
    expect(result.map((item) => item.camera.cameraId)).toEqual(['cam-003'])

    // 每个维度单独都命中，但同时施加时交集为空
    expect(
      filterTasks(entries, { query: '库房', armStatus: 'armed', algorithmId: 'general_detection' }),
    ).toEqual([])
  })

  it('keeps the relative order of surviving entries', () => {
    const result = filterTasks(entries, {
      query: '周界',
      armStatus: 'all',
      algorithmId: ALGORITHM_FILTER_ALL,
    })
    expect(result.map((item) => item.camera.cameraId)).toEqual(['cam-001', 'cam-002'])
  })

  it('returns an empty array when nothing matches', () => {
    const result = filterTasks(entries, {
      query: '不存在的通道',
      armStatus: 'all',
      algorithmId: ALGORITHM_FILTER_ALL,
    })
    expect(result).toEqual([])
  })
})

describe('countByArmStatus', () => {
  it('counts each bucket over the full set, independent of any filter', () => {
    const configs = [
      makeConfig({ desiredEnabled: true }),
      makeConfig({ desiredEnabled: true }),
      makeConfig({ desiredEnabled: false }),
    ]
    expect(countByArmStatus(configs)).toEqual({ all: 3, armed: 2, disarmed: 1 })
  })

  it('keeps buckets summing to the total for an empty list', () => {
    expect(countByArmStatus([])).toEqual({ all: 0, armed: 0, disarmed: 0 })
  })
})

describe('deriveAlgorithmOptions', () => {
  const configs = [
    makeConfig({
      algorithmInstances: [
        { algorithmId: 'general_detection', analysisFps: 10 },
        { algorithmId: 'fire_detections', analysisFps: 5 },
      ],
    }),
    makeConfig({
      algorithmInstances: [{ algorithmId: 'fire_detections', analysisFps: 5 }],
    }),
  ]

  it('deduplicates algorithm IDs across tasks', () => {
    expect(deriveAlgorithmOptions(configs)).toEqual([
      { value: 'fire_detections', label: 'fire_detections' },
      { value: 'general_detection', label: 'general_detection' },
    ])
  })

  it('prefers friendly names and sorts by label', () => {
    // 标签用 ASCII，避免断言依赖运行时的 locale collation 顺序
    const nameById = new Map([
      ['fire_detections', 'Fire Detection'],
      ['general_detection', 'General Detection'],
    ])
    expect(deriveAlgorithmOptions(configs, nameById)).toEqual([
      { value: 'fire_detections', label: 'Fire Detection' },
      { value: 'general_detection', label: 'General Detection' },
    ])
  })

  it('falls back to the raw algorithm ID when the name mapping is missing', () => {
    // 只给得起部分映射（如算法数超过首页返回量）：缺失项回落原始 ID，功能不降级
    const partial = new Map([['fire_detections', 'Fire Detection']])
    expect(deriveAlgorithmOptions(configs, partial)).toEqual([
      { value: 'fire_detections', label: 'Fire Detection' },
      { value: 'general_detection', label: 'general_detection' },
    ])
  })

  it('returns no options when no task mounts an algorithm', () => {
    expect(deriveAlgorithmOptions([makeConfig()])).toEqual([])
  })

  it('ignores blank algorithm IDs', () => {
    const options = deriveAlgorithmOptions([
      makeConfig({ algorithmInstances: [{ algorithmId: '   ', analysisFps: 5 }] }),
    ])
    expect(options).toEqual([])
  })
})

describe('ARM_STATUS_FILTERS', () => {
  it('declares the three arm status buckets with all first', () => {
    expect(ARM_STATUS_FILTERS).toEqual(['all', 'armed', 'disarmed'])
  })
})
