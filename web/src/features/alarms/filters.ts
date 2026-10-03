import type { AlarmStatus, RecognitionStatus } from '@/types'

/**
 * 证据三支柱的筛选维度取值表。
 *
 * 这些联合类型同时被「下拉选项」和「请求参数」消费，因此集中声明一次：
 * 组件只按表渲染，收窄交给 [SelectField](../../../components/ui/SelectField.tsx)，
 * 避免每个调用点各自 `as` 断言（断言在选项表与类型漂移时会静默放行非法值）。
 *
 * 每个维度都由 `defineFilters` 声明，从而与既有联合类型双向锁定：
 * 漏档（枚举新增了成员但表没跟）与越界（表里有联合类型不认识的值）
 * 都在**声明处**直接报错，不需要额外写一个 `Exhaustive` 别名再去别处消费它。
 */

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
 * 柯里化的两层泛型是关键：`TUnion` 在调用处显式给出，`TList` 由实参推断，
 * 因此「双向锁定」的检查落在**实参位置**，写错立即在那一行报错。
 * 若只写 `type X = Exactly<A, typeof Y>`，该别名未被消费时完全不产生约束——
 * 一个恒为真的空断言。
 */
function defineFilters<TUnion extends string>() {
  return <const TList extends readonly string[]>(list: TList & Exactly<TUnion, TList>): TList =>
    list
}

/** 规则类型筛选，与 `alarms.rule_type` 落库取值对齐；`all` 表示不过滤 */
export const RULE_TYPE_FILTERS = defineFilters<'roi' | 'line'>()(['all', 'roi', 'line'])
export type RuleTypeFilter = (typeof RULE_TYPE_FILTERS)[number]

/**
 * 告警处理状态筛选。
 *
 * 与 [RECOGNITION_STATUS_FILTERS] 分开声明：二者取值域完全不同（`unprocessed`/
 * `processed` vs `confirmed`/`pending_review`/`rejected`），合并成一张表会让
 * 「告警状态下拉出现识别状态选项」成为可编译的写法。
 */
export const ALARM_STATUS_FILTERS = defineFilters<AlarmStatus>()([
  'all',
  'unprocessed',
  'processed',
])
export type AlarmStatusFilter = (typeof ALARM_STATUS_FILTERS)[number]

/** 识别对账状态筛选，与 `RecognitionStatus` 全量对齐 */
export const RECOGNITION_STATUS_FILTERS = defineFilters<RecognitionStatus>()([
  'all',
  'confirmed',
  'pending_review',
  'rejected',
])
export type RecognitionStatusFilter = (typeof RECOGNITION_STATUS_FILTERS)[number]

/** 目标类别筛选值；空串表示「全部类别」 */
export type TargetLabelFilter = string

/** 所有筛选下拉共用的「不过滤」档标识 */
export const FILTER_ALL = 'all' as const
