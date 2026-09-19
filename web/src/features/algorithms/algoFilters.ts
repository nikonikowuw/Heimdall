import type { AlgorithmItem, AlgorithmVersionItem } from '@/types'

/** 来源过滤档位；`all` 不做过滤 */
export type AlgoOriginFilter = 'all' | 'builtin' | 'custom'

/** 排序档位；`default` 保持服务端顺序（按主键倒序，最近上传的算法在前） */
export type AlgoSortKey = 'default' | 'name' | 'updated' | 'versions'

export interface AlgoListQuery {
  keyword: string
  /** 算法类型；`all` 不做过滤 */
  algorithmType: string
  origin: AlgoOriginFilter
  /**
   * 归一化平台代号；`all` 不做过滤。
   *
   * 平台是版本的嵌套维度，服务端查询参数没有该维度，因此这一档只在当前页内收敛；
   * 其余档位服务端已下推，前端只做输入期的即时反馈。
   */
  platform: string
  sortKey: AlgoSortKey
}

export const DEFAULT_ALGO_QUERY: AlgoListQuery = {
  keyword: '',
  algorithmType: 'all',
  origin: 'all',
  platform: 'all',
  sortKey: 'default',
}

/**
 * 当前产品已定义的常见算法类型；动态包类型仍会由清单追加到下拉选项。
 * 保留这组稳定选项，避免服务端按已选类型返回后，用户失去切换到其它类型的入口。
 */
export const COMMON_ALGORITHM_TYPES = [
  'object_detection',
  'face_recognition',
  'license_plate_recognition',
] as const

/**
 * 单页算法资产容量上限。
 *
 * 算法是低频变更的配置型资产，边缘设备上通常只有个位数到几十条；
 * 该上限同时与服务端 `list_algorithms` 的硬上限一致，超出部分靠搜索与筛选收敛，
 * 由界面显式告知「共 N 项 / 已显示 M 项」，不做静默截断。
 */
export const ALGO_LIST_PAGE_SIZE = 200

/**
 * 列表视图状态。
 *
 * `error` 与 `empty` 必须严格区分：接口失败时渲染「仓库暂无算法」是把故障说成空数据，
 * 会让运维去排查一个并不存在的问题。
 */
export type AlgoListState = 'loading' | 'error' | 'empty' | 'no-match' | 'ready'

export interface AlgoListStateInput {
  hasError: boolean
  isLoading: boolean
  /** 服务端返回的匹配总数（未经本地收敛），用于区分「仓库为空」与「筛选无命中」 */
  totalCount: number
  /** 当前实际渲染的卡片数 */
  visibleCount: number
}

/**
 * 派生列表视图状态。
 *
 * 优先级：已有内容 > 错误 > 加载中 > 仓库为空 > 筛选无命中。
 * 已有内容优先，是为了让刷新失败时保留上一份可用清单，错误另行以横幅提示而不是清空页面。
 */
export function deriveAlgoListState(input: AlgoListStateInput): AlgoListState {
  const { hasError, isLoading, totalCount, visibleCount } = input
  if (visibleCount > 0) return 'ready'
  if (hasError) return 'error'
  if (isLoading) return 'loading'
  if (totalCount === 0) return 'empty'
  return 'no-match'
}

/**
 * 关键字本地谓词。
 *
 * 服务端按 `algorithmId` / `name` 匹配，本地在此之上放宽到描述字段：
 * 本地谓词必须是服务端谓词的**超集**，只能放宽不能收紧，否则会出现
 * 「服务端返回 5 条、界面只剩 2 条」的鬼影。收紧条件（类型/来源）与后端使用同一等值判断。
 */
export function matchesAlgoKeyword(algo: AlgorithmItem, keyword: string): boolean {
  const kw = keyword.trim().toLowerCase()
  if (!kw) return true
  return (
    algo.algorithmId.toLowerCase().includes(kw) ||
    algo.name.toLowerCase().includes(kw) ||
    algo.description.toLowerCase().includes(kw)
  )
}

/** 算法是否归属指定来源档位 */
export function matchesAlgoOrigin(algo: AlgorithmItem, origin: AlgoOriginFilter): boolean {
  if (origin === 'builtin') return algo.isBuiltin
  if (origin === 'custom') return !algo.isBuiltin
  return true
}

/** 算法是否含指定归一化平台代号下的版本 */
export function matchesAlgoPlatform(algo: AlgorithmItem, platform: string): boolean {
  if (platform === 'all') return true
  return algo.versions.some((version) => version.normalizedPlatformId === platform)
}

/** 按完整查询条件收敛当前页算法清单 */
export function filterAlgorithms(
  algorithms: AlgorithmItem[],
  query: AlgoListQuery,
): AlgorithmItem[] {
  return algorithms.filter((algo) => {
    if (!matchesAlgoKeyword(algo, query.keyword)) return false
    if (query.algorithmType !== 'all' && algo.algorithmType !== query.algorithmType) return false
    if (!matchesAlgoOrigin(algo, query.origin)) return false
    if (!matchesAlgoPlatform(algo, query.platform)) return false
    return true
  })
}

/**
 * 排序算法清单（非原地，返回新数组）。
 *
 * 除名称外都作了稳定兜底比较，避免等值项在多次渲染间无规则跳动。
 */
export function sortAlgorithms(algorithms: AlgorithmItem[], sortKey: AlgoSortKey): AlgorithmItem[] {
  const sorted = [...algorithms]
  switch (sortKey) {
    case 'name':
      sorted.sort(
        (a, b) => a.name.localeCompare(b.name) || a.algorithmId.localeCompare(b.algorithmId),
      )
      break
    case 'updated':
      sorted.sort((a, b) => b.updatedAt - a.updatedAt || a.algorithmId.localeCompare(b.algorithmId))
      break
    case 'versions':
      sorted.sort(
        (a, b) =>
          b.versions.length - a.versions.length || a.algorithmId.localeCompare(b.algorithmId),
      )
      break
    case 'default':
      break
  }
  return sorted
}

/** 收集当前清单中实际存在的算法类型（升序去重），避免筛选项硬编码后漏掉新增类型 */
export function collectAlgorithmTypes(algorithms: AlgorithmItem[]): string[] {
  const types = new Set<string>()
  for (const algo of algorithms) {
    const type = algo.algorithmType.trim()
    if (type) types.add(type)
  }
  return [...types].sort()
}

/** 收集当前清单中实际存在的归一化平台代号（升序去重） */
export function collectPlatformOptions(algorithms: AlgorithmItem[]): string[] {
  const platforms = new Set<string>()
  for (const algo of algorithms) {
    for (const version of algo.versions) {
      const platform = version.normalizedPlatformId.trim()
      if (platform) platforms.add(platform)
    }
  }
  return [...platforms].sort()
}

/**
 * 补齐下拉选项中的当前选中值。
 *
 * 选项由**筛选后**的清单派生：一旦当前条件零命中，选项表会丢掉正在生效的值，
 * 原生 select 会退而显示第一个选项（“全部”），于是控件显示与真实筛选状态不符。
 * 保留选中值可消除这种不一致，并保证用户能看见自己选中了什么。
 */
export function withSelectedOption(options: string[], selected: string): string[] {
  if (selected === 'all' || options.includes(selected)) return options
  return [...options, selected].sort()
}

/** 同一 is_active 标记下可能并存多平台行，宿主兼容的那一行才是当前真正生效的二进制 */
function pickHostPreferred(versions: AlgorithmVersionItem[]): AlgorithmVersionItem | undefined {
  return versions.find((version) => version.compatibleWithHost) ?? versions[0]
}

/** 解析算法当前生效版本；与后端 `activeVersion` 字段互为兜底 */
export function activeVersionItem(algo: AlgorithmItem): AlgorithmVersionItem | undefined {
  const flagged = pickHostPreferred(algo.versions.filter((version) => version.isActive))
  if (flagged) return flagged

  const byVersionString = pickHostPreferred(
    algo.versions.filter((version) => version.version === algo.activeVersion),
  )
  if (byVersionString) return byVersionString

  return algo.versions[0]
}

/** 适配当前宿主平台的版本数 */
export function compatibleVersionCount(algo: AlgorithmItem): number {
  return algo.versions.filter((version) => version.compatibleWithHost).length
}

/** 算法是否存在可加载到当前宿主的版本 */
export function hasCompatibleVersion(algo: AlgorithmItem): boolean {
  return algo.versions.some((version) => version.compatibleWithHost)
}

/** 当前是否存在任一非默认筛选条件（用于「清除筛选」入口的可用性） */
export function hasActiveFilters(query: AlgoListQuery): boolean {
  return (
    query.keyword.trim() !== '' ||
    query.algorithmType !== 'all' ||
    query.origin !== 'all' ||
    query.platform !== 'all'
  )
}

/**
 * 合并分页追加结果，按算法 ID 去重。
 *
 * 分页窗口是偏移分页：加载下一页期间若仓库新增了算法，后一页可能与前一页重叠，
 * 去重可以避免同一张卡片出现两次。
 */
export function mergeAlgorithmPages(
  current: AlgorithmItem[],
  incoming: AlgorithmItem[],
): AlgorithmItem[] {
  const seen = new Set(current.map((algo) => algo.algorithmId))
  return [...current, ...incoming.filter((algo) => !seen.has(algo.algorithmId))]
}
