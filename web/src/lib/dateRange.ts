/**
 * 时间区间模型与解析 —— alarms 与 oplog 共用的「时间范围」语义。
 *
 * 与 `lib/time.ts` 的分工：`time.ts` 只做时间**格式化**，且明确禁止用 Date 做
 * 格式化以外的计算；本模块负责时间**区间解析**，包含必要的 Date 运算。
 *
 * 该模块为 UI 时间选择器与其解析逻辑的共同下界，因此不依赖 components/ 或任何 feature。
 */

export type QuickTimePreset =
  'all' | '5m' | '15m' | '30m' | '1h' | '24h' | 'today' | '7d' | 'custom'

export interface DateTimeRangeValue {
  startTime?: number // 13位 UTC 毫秒时间戳
  endTime?: number // 13位 UTC 毫秒时间戳
  quickPreset: QuickTimePreset
}

/** 相对时间预设的回溯窗口（毫秒）；'all' / 'today' / 'custom' 不在此表内 */
export const PRESET_DURATIONS_MS: Partial<Record<QuickTimePreset, number>> = {
  '5m': 5 * 60_000,
  '15m': 15 * 60_000,
  '30m': 30 * 60_000,
  '1h': 3_600_000,
  '24h': 86_400_000,
  '7d': 7 * 86_400_000,
}

/**
 * 实时解析时间范围值中的实际起止 UTC 毫秒时间戳
 * - 对于 'all'：无起止时间限制（均为 undefined）
 * - 对于 'custom'：严格遵循指定的 startTime 与 endTime
 * - 对于 'today'：起始为当天 00:00:00，截止为 undefined（无上限截断，确保新数据可被查出）
 * - 对于相对时间预设（'5m', '15m', '30m', '1h', '24h', '7d'）：基于当前时刻 now 动态回溯滑动窗口
 *
 * `now` 可注入，便于测试与保证同一操作内的多次解析取到一致窗口。
 */
export function resolveEffectiveTimeRange(
  range: DateTimeRangeValue,
  now = Date.now(),
): {
  startTime?: number
  endTime?: number
} {
  if (range.quickPreset === 'all') {
    return { startTime: undefined, endTime: undefined }
  }

  if (range.quickPreset === 'custom') {
    return { startTime: range.startTime, endTime: range.endTime }
  }

  if (range.quickPreset === 'today') {
    const todayStart = new Date(now)
    todayStart.setHours(0, 0, 0, 0)
    return {
      startTime: todayStart.getTime(),
      endTime: undefined,
    }
  }

  const duration = PRESET_DURATIONS_MS[range.quickPreset]
  if (duration) {
    return {
      startTime: now - duration,
      endTime: now,
    }
  }

  return { startTime: range.startTime, endTime: range.endTime }
}

/**
 * 仅按预设求起止时间（供选择器即时回填 draft 值）。
 *
 * 刻意共用 `resolveEffectiveTimeRange`：此前选择器与解析器各自维护一份预设表并
 * 各自取 `Date.now()`，同一预设会算出略有差异的边界；此处收敛为单一实现。
 */
export function resolvePresetTimestamps(
  preset: QuickTimePreset,
  now = Date.now(),
): {
  startTime?: number
  endTime?: number
} {
  return resolveEffectiveTimeRange({ quickPreset: preset }, now)
}
