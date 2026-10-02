import type { Camera, TaskConfigDto } from '@/types'

/**
 * 任务列表检索与筛选的纯逻辑层。
 *
 * 三个维度（文本 / 布防状态 / 算法）为 **AND** 关系，每个维度内部为 **OR**
 * （文本命中任务名、摄像头名、摄像头 ID 任一即算命中）。全部为本地过滤，
 * 不引入服务端查询参数：30 任务规模下服务端过滤无收益，且按通道名检索需要
 * `LEFT JOIN cameras`（非唯一列、无索引），只在客户端真的传 `q` 时才值得拼入。
 *
 * 纯逻辑只消费 `@/types`，不 import 任何 feature：算法名映射由调用方（`TasksPage`）
 * 从 `algorithmApi.list()` 取回后作为参数注入，拿不到时回落原始 `algorithmId`。
 */

function includesNormalizedQuery(
  value: string | null | undefined,
  normalizedQuery: string,
): boolean {
  return value?.toLowerCase().includes(normalizedQuery) ?? false
}

/** 查询串归一化：trim + lowercase。供调用方在循环外算一次，避免每条任务重复归一化。 */
export function normalizeTaskQuery(query: string): string {
  return query.trim().toLowerCase()
}

/**
 * 文本匹配：命中任务名、摄像头名、摄像头 ID 任一即通过。
 *
 * @param query         原始查询串
 * @param preNormalized 已归一化的查询串；传空串等价于「无查询」
 * @param camera        该任务对应的摄像头；数据缺口（找不到摄像头记录）时仅按任务名匹配，不误伤
 */
export function matchesTaskQuery(
  config: TaskConfigDto,
  camera: Camera | undefined,
  query: string,
  preNormalized?: string,
): boolean {
  const normalizedQuery = preNormalized ?? normalizeTaskQuery(query)
  if (!normalizedQuery) return true

  return (
    includesNormalizedQuery(config.name, normalizedQuery) ||
    includesNormalizedQuery(camera?.name, normalizedQuery) ||
    includesNormalizedQuery(camera?.cameraId, normalizedQuery)
  )
}

/**
 * 选项表须恰好覆盖 `TUnion`：先 `Exclude` 再要求 `never`。
 *
 * 仅用 `satisfies readonly ['all', ...T[]]` 只能拦住越界；某个枚举成员从常量中
 * 被意外删除时它毫无异议，而这正是下拉会静默丢选项的原因。
 * 两个方向的检查并列为元组一次性判定，避免嵌套条件类型。
 */
type Exactly<TUnion extends string, TList extends readonly string[]> = [
  Exclude<TUnion, TList[number]>,
  Exclude<TList[number], TUnion | 'all'>,
] extends [never, never]
  ? unknown
  : never

/**
 * 声明一个筛选维度的取值表。
 *
 * **本地复刻**自 `features/alarms/filters.ts` 的同名实现，而非跨域导入：那是 alarms
 * 域的私有实现，深层导入违反模块边界规范。本地复刻（约 10 行泛型技巧）而不是上提
 * `lib/`，是因为上提需要改动 alarms 既有代码并扩大 diff，而当前只有 alarms 与 tasks
 * 两个消费点，尚不满足「两个以上 feature 实际共用」的上提判据。
 * 第三个消费点出现时应上提至 `lib/`，并同步迁移两处调用（取舍见
 * `.trellis/spec/web/frontend/component-guidelines.md#列表检索与筛选`）。
 *
 * 柯里化的两层泛型是关键：`TUnion` 在调用处显式给出，`TList` 由实参推断，
 * 因此「双向锁定」的检查落在**实参位置**，写错立即在那一行报错。
 */
function defineFilters<TUnion extends string>() {
  return <const TList extends readonly string[]>(list: TList & Exactly<TUnion, TList>): TList =>
    list
}

/** 布防状态维度取值表。判定字段为 `desiredEnabled`，与页面 KPI 的 `armedCount` 同源 */
export const ARM_STATUS_FILTERS = defineFilters<'armed' | 'disarmed'>()([
  'all',
  'armed',
  'disarmed',
])
export type ArmStatusFilter = (typeof ARM_STATUS_FILTERS)[number]

/**
 * 布防状态各档位的计数。键集与 `ArmStatusFilter` 同集，因此可按档位直接下标取用
 * （`counts[option]`），无需在渲染处再写一遍档位到字段的映射。
 *
 * `all` 恒等于 `armed + disarmed`，由 [countByArmStatus] 保证。
 */
export type ArmStatusCounts = Record<ArmStatusFilter, number>

/** 算法维度的「不过滤」档，与 `ARM_STATUS_FILTERS` 的 `all` 档同义 */
export const ALGORITHM_FILTER_ALL = 'all'

/**
 * 布防状态维度匹配：判定字段为 `desiredEnabled`，与页面 KPI 同源。
 *
 * 传入 `'all'` 表示不过滤。
 */
export function matchesTaskArmStatus(config: TaskConfigDto, armStatus: ArmStatusFilter): boolean {
  return armStatus === 'all' || (config.desiredEnabled ? 'armed' : 'disarmed') === armStatus
}

/**
 * 算法维度匹配：任务挂载的实例中**任一** `algorithmId` 命中即通过。
 *
 * 多实例任务（如同时挂载通用检测与火焰检测）在筛「火焰检测」时必须被命中，
 * 否则用户会得到静默的错误答案。实例数组缺失（`undefined`）时视为不匹配任何
 * 具体算法，但在 `ALGORITHM_FILTER_ALL` 下照常通过。
 */
export function matchesTaskAlgorithm(config: TaskConfigDto, selectedAlgorithmId: string): boolean {
  if (selectedAlgorithmId === ALGORITHM_FILTER_ALL) return true

  return (config.algorithmInstances ?? []).some(
    (instance) => instance.algorithmId === selectedAlgorithmId,
  )
}

/** 一个「摄像头通道 + 任务配置」的检索单元；顺序即 `cameras` 数组顺序（D6） */
export interface TaskFilterEntry {
  camera: Camera
  config: TaskConfigDto
}

export interface TaskFilters {
  query: string
  armStatus: ArmStatusFilter
  algorithmId: string
}

/**
 * 是否存在任一非默认筛选条件（三维度的唯一判定处）。
 *
 * 筛选栏据它决定是否展示「清除筛选」，因此不能与 [filterTasks] 的短路条件各写
 * 一份：两处漂移会导致「按钮不出现但列表已被过滤」这类自相矛盾的界面。
 */
export function hasActiveTaskFilters(filters: TaskFilters): boolean {
  return (
    normalizeTaskQuery(filters.query) !== '' ||
    filters.armStatus !== 'all' ||
    filters.algorithmId !== ALGORITHM_FILTER_ALL
  )
}

/**
 * 三维度组合过滤（AND）。
 *
 * 无筛选时**返回入参原引用**：让下游 `useMemo` 的依赖变化可被察觉，避免每次渲染
 * 产生新数组引用而触发卡片矩阵的全量重渲染。有筛选时是顺序保持过滤
 * （`Array.prototype.filter` 语义），筛选后卡片的相对先后与筛选前一致，只有被
 * 滤除的项消失 —— 本次不做稳定排序干预，也不重排 `cameras`。
 */
export function filterTasks(
  entries: ReadonlyArray<TaskFilterEntry>,
  filters: TaskFilters,
): ReadonlyArray<TaskFilterEntry> {
  if (!hasActiveTaskFilters(filters)) return entries

  const normalizedQuery = normalizeTaskQuery(filters.query)
  return entries.filter(
    (entry) =>
      matchesTaskQuery(entry.config, entry.camera, filters.query, normalizedQuery) &&
      matchesTaskArmStatus(entry.config, filters.armStatus) &&
      matchesTaskAlgorithm(entry.config, filters.algorithmId),
  )
}

/**
 * 各档位计数（**全量口径**，不随当前筛选变化）。
 *
 * 筛选栏内的计数与页面 KPI 一样始终描述整个任务集，命中数另行展示；
 * 否则「已布防 15」会随文本检索一起缩水，用户无法判断筛选前后分布。
 */
export function countByArmStatus(configs: ReadonlyArray<TaskConfigDto>): ArmStatusCounts {
  let armed = 0
  for (const config of configs) {
    if (config.desiredEnabled) armed += 1
  }
  return { all: configs.length, armed, disarmed: configs.length - armed }
}

/**
 * 从已加载任务派生 distinct `algorithmId`，按友好名排序。
 *
 * 选项派生自**实际在用**集合，因此每个选项必然有结果，不会出现选中后空列表。
 * 友好名映射缺失（`algorithmApi.list()` 失败或算法数超出首页返回量）时回落原始
 * `algorithmId`，功能不降级。
 */
export function deriveAlgorithmOptions(
  configs: ReadonlyArray<TaskConfigDto>,
  nameById?: ReadonlyMap<string, string>,
): Array<{ value: string; label: string }> {
  const algorithmIds = new Set<string>()
  for (const config of configs) {
    for (const instance of config.algorithmInstances ?? []) {
      const algorithmId = instance.algorithmId?.trim()
      if (algorithmId) algorithmIds.add(algorithmId)
    }
  }

  return [...algorithmIds]
    .map((algorithmId) => ({
      value: algorithmId,
      label: nameById?.get(algorithmId) || algorithmId,
    }))
    .sort((a, b) => a.label.localeCompare(b.label))
}
