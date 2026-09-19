import { describe, expect, it } from 'vitest'
import type { AlgorithmItem, AlgorithmVersionItem } from '@/types'
import {
  activeVersionItem,
  collectAlgorithmTypes,
  collectPlatformOptions,
  compatibleVersionCount,
  DEFAULT_ALGO_QUERY,
  deriveAlgoListState,
  filterAlgorithms,
  hasActiveFilters,
  hasCompatibleVersion,
  matchesAlgoKeyword,
  sortAlgorithms,
  withSelectedOption,
  type AlgoListQuery,
} from './algoFilters'

function makeVersion(overrides: Partial<AlgorithmVersionItem> = {}): AlgorithmVersionItem {
  const platformId = overrides.platformId ?? 'rknn'
  return {
    id: 1,
    algorithmId: 'general_detection',
    version: '1.0.0',
    platformId,
    normalizedPlatformId: overrides.normalizedPlatformId ?? 'linux-rknn',
    compatibleWithHost: overrides.compatibleWithHost ?? false,
    minAdapterVersion: '1.0.0',
    packageRoot: 'var/packages/general_detection/1.0.0',
    fpsTiers: [],
    configSchema: {},
    manifestRaw: {},
    packageSizeBytes: 1024,
    isActive: false,
    isBuiltin: true,
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
    ...overrides,
  }
}

function makeAlgo(overrides: Partial<AlgorithmItem> = {}): AlgorithmItem {
  return {
    id: 1,
    algorithmId: 'general_detection',
    name: '通用目标检测',
    algorithmType: 'object_detection',
    alarmTypeId: 'object_detect',
    activeVersion: '1.0.0',
    description: '通用检测模型',
    isBuiltin: true,
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
    versions: [makeVersion()],
    ...overrides,
  }
}

function query(overrides: Partial<AlgoListQuery> = {}): AlgoListQuery {
  return { ...DEFAULT_ALGO_QUERY, ...overrides }
}

describe('matchesAlgoKeyword', () => {
  const algo = makeAlgo({
    algorithmId: 'PlateNet',
    name: '车牌识别',
    description: '基于 YOLO 的车牌检测与识别',
  })

  it('空关键字透传', () => {
    expect(matchesAlgoKeyword(algo, '')).toBe(true)
    expect(matchesAlgoKeyword(algo, '   ')).toBe(true)
  })

  it('命中算法 ID / 名称 / 描述且忽略大小写', () => {
    expect(matchesAlgoKeyword(algo, 'platenet')).toBe(true)
    expect(matchesAlgoKeyword(algo, '车牌')).toBe(true)
    expect(matchesAlgoKeyword(algo, 'yolo')).toBe(true)
  })

  it('未命中任何字段时返回 false', () => {
    expect(matchesAlgoKeyword(algo, 'face')).toBe(false)
  })

  it('本地谓词是服务端（ID/名称）谓词的超集，不会吞掉服务端已命中的结果', () => {
    // 服务端仅按 algorithmId/name 匹配，本地断言必须对同一批关键字同样命中
    for (const keyword of ['PlateNet', '车牌识别', 'PLATENET']) {
      expect(matchesAlgoKeyword(algo, keyword)).toBe(true)
    }
  })
})

describe('filterAlgorithms', () => {
  const detection = makeAlgo({
    id: 1,
    algorithmId: 'general_detection',
    name: '通用目标检测',
    algorithmType: 'object_detection',
    versions: [
      makeVersion({ normalizedPlatformId: 'macos-arm64', platformId: 'macos-arm64-coreml' }),
    ],
  })
  const face = makeAlgo({
    id: 2,
    algorithmId: 'face_recognition',
    name: '人脸识别',
    algorithmType: 'face_recognition',
    isBuiltin: false,
    versions: [makeVersion({ normalizedPlatformId: 'linux-rknn' })],
  })
  const plate = makeAlgo({
    id: 3,
    algorithmId: 'plate_recognition',
    name: '车牌识别',
    algorithmType: 'license_plate_recognition',
    versions: [
      makeVersion({ normalizedPlatformId: 'linux-rknn' }),
      makeVersion({ normalizedPlatformId: 'macos-arm64', version: '2.0.0' }),
    ],
  })
  const all = [detection, face, plate]

  it('默认查询保留全部算法', () => {
    expect(filterAlgorithms(all, query())).toHaveLength(3)
  })

  it('按算法类型精确过滤', () => {
    const result = filterAlgorithms(all, query({ algorithmType: 'face_recognition' }))
    expect(result.map((a) => a.algorithmId)).toEqual(['face_recognition'])
  })

  it('按来源档位过滤', () => {
    expect(filterAlgorithms(all, query({ origin: 'builtin' })).map((a) => a.algorithmId)).toEqual([
      'general_detection',
      'plate_recognition',
    ])
    expect(filterAlgorithms(all, query({ origin: 'custom' })).map((a) => a.algorithmId)).toEqual([
      'face_recognition',
    ])
  })

  it('按归一化平台过滤（穿透 macos-arm64-coreml 别名）', () => {
    const result = filterAlgorithms(all, query({ platform: 'macos-arm64' }))
    expect(result.map((a) => a.algorithmId)).toEqual(['general_detection', 'plate_recognition'])
  })

  it('多档位同时生效', () => {
    const result = filterAlgorithms(
      all,
      query({ keyword: '识别', platform: 'linux-rknn', origin: 'custom' }),
    )
    expect(result.map((a) => a.algorithmId)).toEqual(['face_recognition'])
  })
})

describe('sortAlgorithms', () => {
  const alpha = makeAlgo({
    id: 1,
    algorithmId: 'alpha',
    name: 'B 模型',
    updatedAt: 300,
    versions: [makeVersion()],
  })
  const beta = makeAlgo({
    id: 2,
    algorithmId: 'beta',
    name: 'A 模型',
    updatedAt: 100,
    versions: [makeVersion({ id: 1 }), makeVersion({ id: 2 })],
  })
  const gamma = makeAlgo({
    id: 3,
    algorithmId: 'gamma',
    name: 'A 模型',
    updatedAt: 200,
    versions: [makeVersion()],
  })
  const list = [alpha, beta, gamma]

  it('default 保持服务端顺序', () => {
    expect(sortAlgorithms(list, 'default').map((a) => a.algorithmId)).toEqual([
      'alpha',
      'beta',
      'gamma',
    ])
  })

  it('name 升序并以算法 ID 兜底，避免同名项乱序', () => {
    expect(sortAlgorithms(list, 'name').map((a) => a.algorithmId)).toEqual([
      'beta',
      'gamma',
      'alpha',
    ])
  })

  it('updated 按更新时间倒序', () => {
    expect(sortAlgorithms(list, 'updated').map((a) => a.algorithmId)).toEqual([
      'alpha',
      'gamma',
      'beta',
    ])
  })

  it('versions 按版本数倒序', () => {
    expect(sortAlgorithms(list, 'versions').map((a) => a.algorithmId)).toEqual([
      'beta',
      'alpha',
      'gamma',
    ])
  })

  it('不修改入参数组', () => {
    const snapshot = list.map((a) => a.algorithmId)
    sortAlgorithms(list, 'name')
    expect(list.map((a) => a.algorithmId)).toEqual(snapshot)
  })
})

describe('collectAlgorithmTypes / collectPlatformOptions', () => {
  it('去重并升序返回实际存在的类型与平台', () => {
    const list = [
      makeAlgo({
        id: 1,
        algorithmType: 'object_detection',
        versions: [makeVersion({ normalizedPlatformId: 'linux-rknn' })],
      }),
      makeAlgo({
        id: 2,
        algorithmType: 'face_recognition',
        versions: [makeVersion({ normalizedPlatformId: 'macos-arm64' })],
      }),
      makeAlgo({
        id: 3,
        algorithmType: 'object_detection',
        versions: [
          makeVersion({ normalizedPlatformId: 'macos-arm64' }),
          makeVersion({ normalizedPlatformId: 'linux-rknn' }),
        ],
      }),
    ]
    expect(collectAlgorithmTypes(list)).toEqual(['face_recognition', 'object_detection'])
    expect(collectPlatformOptions(list)).toEqual(['linux-rknn', 'macos-arm64'])
  })

  it('空清单返回空选项', () => {
    expect(collectAlgorithmTypes([])).toEqual([])
    expect(collectPlatformOptions([])).toEqual([])
  })
})

describe('deriveAlgoListState', () => {
  it('已有内容时优先呈现内容，刷新失败也不清空页面', () => {
    expect(
      deriveAlgoListState({ hasError: true, isLoading: false, totalCount: 5, visibleCount: 2 }),
    ).toBe('ready')
  })

  it('无内容且请求失败时为错误态，而不是「没有算法」', () => {
    expect(
      deriveAlgoListState({ hasError: true, isLoading: false, totalCount: 0, visibleCount: 0 }),
    ).toBe('error')
  })

  it('加载中且尚无内容时为骨架态', () => {
    expect(
      deriveAlgoListState({ hasError: false, isLoading: true, totalCount: 0, visibleCount: 0 }),
    ).toBe('loading')
  })

  it('仓库本身为空时为空态', () => {
    expect(
      deriveAlgoListState({ hasError: false, isLoading: false, totalCount: 0, visibleCount: 0 }),
    ).toBe('empty')
  })

  it('有数据但被筛选挡住时为无命中态', () => {
    expect(
      deriveAlgoListState({ hasError: false, isLoading: false, totalCount: 12, visibleCount: 0 }),
    ).toBe('no-match')
  })

  it('错误态优先于筛选无命中，避免把故障误报成筛选问题', () => {
    expect(
      deriveAlgoListState({ hasError: true, isLoading: false, totalCount: 12, visibleCount: 0 }),
    ).toBe('error')
  })
})

describe('activeVersionItem', () => {
  it('优先取 isActive 标记，并在多平台并存时选宿主兼容行', () => {
    const algo = makeAlgo({
      versions: [
        makeVersion({
          id: 1,
          platformId: 'rknn',
          normalizedPlatformId: 'linux-rknn',
          isActive: true,
        }),
        makeVersion({
          id: 2,
          platformId: 'macos-arm64',
          normalizedPlatformId: 'macos-arm64',
          compatibleWithHost: true,
          isActive: true,
        }),
      ],
    })
    expect(activeVersionItem(algo)?.id).toBe(2)
  })

  it('无 isActive 标记时按 activeVersion 字符串匹配', () => {
    const algo = makeAlgo({
      activeVersion: '2.0.0',
      versions: [
        makeVersion({ id: 1, version: '1.0.0' }),
        makeVersion({ id: 2, version: '2.0.0' }),
      ],
    })
    expect(activeVersionItem(algo)?.id).toBe(2)
  })

  it('两项都落空时兜底首个版本', () => {
    const algo = makeAlgo({
      activeVersion: '',
      versions: [makeVersion({ id: 7 }), makeVersion({ id: 8 })],
    })
    expect(activeVersionItem(algo)?.id).toBe(7)
  })

  it('无版本时返回 undefined', () => {
    expect(activeVersionItem(makeAlgo({ versions: [] }))).toBeUndefined()
  })
})

describe('兼容性与筛选可用性', () => {
  it('统计适配当前宿主的版本数', () => {
    const algo = makeAlgo({
      versions: [
        makeVersion({ compatibleWithHost: true }),
        makeVersion({ compatibleWithHost: true }),
        makeVersion({ compatibleWithHost: false }),
      ],
    })
    expect(compatibleVersionCount(algo)).toBe(2)
    expect(hasCompatibleVersion(algo)).toBe(true)
  })

  it('全部版本都不适配时为 false', () => {
    const algo = makeAlgo({ versions: [makeVersion({ compatibleWithHost: false })] })
    expect(compatibleVersionCount(algo)).toBe(0)
    expect(hasCompatibleVersion(algo)).toBe(false)
  })

  it('仅空白关键字不算激活筛选', () => {
    expect(hasActiveFilters(query({ keyword: '  ' }))).toBe(false)
    expect(hasActiveFilters(query({ origin: 'custom' }))).toBe(true)
    expect(hasActiveFilters(query({ platform: 'linux-rknn' }))).toBe(true)
  })
})

describe('withSelectedOption', () => {
  it('选项已包含选中值时保持原样', () => {
    const options = ['linux-rknn', 'macos-arm64']
    expect(withSelectedOption(options, 'linux-rknn')).toBe(options)
  })

  it('零命中导致选项表丢掉选中值时补齐并保持升序', () => {
    expect(withSelectedOption(['macos-arm64'], 'linux-rknn')).toEqual(['linux-rknn', 'macos-arm64'])
  })

  it('哨兵值 all 不作为具体选项插入', () => {
    expect(withSelectedOption(['linux-rknn'], 'all')).toEqual(['linux-rknn'])
  })

  it('空选项表也能保住选中值', () => {
    expect(withSelectedOption([], 'face_recognition')).toEqual(['face_recognition'])
  })
})
