import type { OperationalLogLevel } from '@/types'

/** 操作审计的业务模块，与审计中间件写入 operation_logs.module 的取值对齐 */
export const MODULE_FILTERS = [
  'all',
  'auth',
  'camera',
  'task',
  'task_instance',
  'algorithm',
  'alarm',
  'evidence',
  'system',
] as const
export type ModuleFilter = (typeof MODULE_FILTERS)[number]

/** 状态码分类筛选，与 GET /logs/operations 的 status 参数取值一一对应 */
export const STATUS_FILTERS = ['all', 'success', 'failed'] as const
export type StatusFilter = (typeof STATUS_FILTERS)[number]

/** 运维事件级别，与服务端 level 参数及 OpEvent::level 的取值对齐 */
export const LEVEL_VALUES = [
  'info',
  'warn',
  'error',
] as const satisfies readonly OperationalLogLevel[]

/** `all` 表示不过滤 */
export type LevelFilter = 'all' | OperationalLogLevel
export const LEVEL_FILTERS: readonly LevelFilter[] = ['all', ...LEVEL_VALUES]

/** 运维事件归属模块，与 crates/types/src/oplog.rs 的 OpEvent::target 对齐 */
export const TARGET_FILTERS = [
  'all',
  'system',
  'media',
  'pipeline',
  'rule',
  'infer',
  'hardware',
  'storage',
] as const
export type TargetFilter = (typeof TARGET_FILTERS)[number]

/**
 * 运维事件标记，与 crates/types/src/oplog.rs 的 OpEvent::tag 逐项对齐；
 * 服务端 event 参数为精确匹配，因此这里列出全部已知事件。
 */
export const EVENT_FILTERS = [
  'all',
  'service_started',
  'service_stopped',
  'camera_online',
  'camera_offline',
  'task_started',
  'task_stopped',
  'task_completed',
  'task_failed',
  'task_degraded',
  'alarm_triggered',
  'algo_loaded',
  'algo_unloaded',
  'algo_load_failed',
  'algo_sandbox_failed',
  'npu_init_failed',
  'vpu_exhausted',
  'storage_watermark',
  'storage_eviction',
] as const
export type EventFilter = (typeof EVENT_FILTERS)[number]

/**
 * `<select>` 等原生控件的 value 只会给出 `string`，收窄回字面量联合必须靠运行时校验，
 * 不接受 `as` 断言。
 */
export function isMemberOf<T extends string>(options: readonly T[], value: string): value is T {
  return (options as readonly string[]).includes(value)
}
