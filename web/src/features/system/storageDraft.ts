import type { StorageConfig } from '@/types/system'

/**
 * 存储策略草稿的数值区间与保存兜底收敛（存储页站点适配层，纯逻辑）。
 *
 * 三段式契约见 .trellis/spec/web/frontend/component-guidelines.md「数值输入与校验时机」：
 * 输入框负责编辑期草稿与失焦收敛（`NumericField`），保存路径必须独立再收敛一次（R3），
 * 不假设 UI 已收敛。区间与 `crates/types/src/system.rs` 的 `StorageConfig::validate()` 同源：
 * 保留天数 1–365、批删除数 10–500；配额 `0 = 不限`（下界 0、无上界）；
 * 水位是 0–1 的比值，UI 以百分数呈现，换算只在站点层做（组件只见百分数）。
 *
 * 参考先例：features/cameras/recordingConfig.ts#clampRecordingConfig。
 */
export const STORAGE_LIMITS = {
  retentionDays: { min: 1, max: 365 },
  /** 配额下界 0 表示「不限」，无上界 */
  quotaMb: { min: 0 },
  /** DTO 形态的水位比值区间 */
  freeRatio: { min: 0, max: 1 },
  batchDeleteSize: { min: 10, max: 500 },
} as const

/** 水位输入框呈现的百分数区间，与 `freeRatio` 的 0–1 一一对应 */
export const FREE_RATIO_PERCENT_LIMITS = { min: 0, max: 100 } as const

/** 后端 `StorageConfig::default()` 的事件录像保留天数；旧服务端省略该字段时的兜底 */
export const DEFAULT_RECORDING_RETENTION_DAYS = 7

/**
 * 存储策略草稿：共享 DTO 加上后端实际存在、但 web 尚未声明的字段。
 *
 * `crates/types/src/system.rs` 的 `StorageConfig` 已包含 `recording_retention_days`
 * （`serde(default = 7)`，且 `validate()` 把它限制在 1–365），GET 响应与 PUT 请求
 * 双向都会带它；web 共享 DTO `types/system.ts` 漏声明了这个字段。
 * 本页没有对应的界面控件，但保存是整对象覆盖，必须原样带回：省略会被 serde default
 * 静默重置为 7，抹掉用户在别处设过的值。
 *
 * 局部扩展而不是直接补 DTO：本次任务是输入校验契约，补全共享 DTO 会外溢到类型边界；
 * 待有独立控件接入时再把字段上提。
 */
export interface StorageDraft extends StorageConfig {
  recordingRetentionDays?: number
}

/** 整数语义字段收敛：非有限值回落区间下界，其余取整后夹到区间内 */
function clampInteger(value: number, range: { min: number; max?: number }): number {
  if (!Number.isFinite(value)) return range.min
  const rounded = Math.round(value)
  const withMinimum = Math.max(range.min, rounded)
  return range.max === undefined ? withMinimum : Math.min(range.max, withMinimum)
}

/** 比值语义字段收敛：非有限值回落区间下界；不取整，否则 0.15 会被折成 0 */
function clampRatio(value: number, range: { min: number; max: number }): number {
  if (!Number.isFinite(value)) return range.min
  return Math.min(range.max, Math.max(range.min, value))
}

/** 水位比值 → 输入框百分数 */
export function ratioToPercent(ratio: number): number {
  return Math.round(ratio * 100)
}

/** 输入框百分数 → 水位比值 */
export function percentToRatio(percent: number): number {
  return percent / 100
}

/**
 * 保存前的最终收敛（R3）：对全部数值字段夹到合法区间，其它字段原样透传。
 *
 * 越界的可见字段不收敛会被后端 `validate()` 整体拒绝（51100），用户无法从界面
 * 判断哪一项越界；`recordingRetentionDays` 虽无输入框，但它与其余保留天数一样受
 * `1–365` 校验，同样必须收敛。
 */
export function clampStorageDraft(draft: StorageDraft): StorageDraft {
  return {
    ...draft,
    alarmRetentionDays: clampInteger(draft.alarmRetentionDays, STORAGE_LIMITS.retentionDays),
    recognitionRetentionDays: clampInteger(
      draft.recognitionRetentionDays,
      STORAGE_LIMITS.retentionDays,
    ),
    captureRetentionDays: clampInteger(draft.captureRetentionDays, STORAGE_LIMITS.retentionDays),
    // 声明为可选是兼容旧服务端：字段缺失时不能走 `Number.isFinite` 分支被夹成 1，
    // 那会把事件录像保留策略意外收紧成一天；缺失按 `serde(default)` 的 7 兜底。
    recordingRetentionDays: clampInteger(
      draft.recordingRetentionDays ?? DEFAULT_RECORDING_RETENTION_DAYS,
      STORAGE_LIMITS.retentionDays,
    ),
    alarmQuotaMb: clampInteger(draft.alarmQuotaMb, STORAGE_LIMITS.quotaMb),
    recognitionQuotaMb: clampInteger(draft.recognitionQuotaMb, STORAGE_LIMITS.quotaMb),
    captureQuotaMb: clampInteger(draft.captureQuotaMb, STORAGE_LIMITS.quotaMb),
    minFreeRatio: clampRatio(draft.minFreeRatio, STORAGE_LIMITS.freeRatio),
    targetFreeRatio: clampRatio(draft.targetFreeRatio, STORAGE_LIMITS.freeRatio),
    emergencyFreeRatio: clampRatio(draft.emergencyFreeRatio, STORAGE_LIMITS.freeRatio),
    criticalFreeRatio: clampRatio(draft.criticalFreeRatio, STORAGE_LIMITS.freeRatio),
    batchDeleteSize: clampInteger(draft.batchDeleteSize, STORAGE_LIMITS.batchDeleteSize),
  }
}
