import type { CameraRecordingConfig } from '@/types'

/** 未配置录像时的默认值，与后端 `CameraRecordingConfig::default()` 对齐 */
export const DEFAULT_RECORDING_CONFIG: CameraRecordingConfig = {
  enabled: false,
  preCaptureSeconds: 10,
  postCaptureSeconds: 10,
  maxFileSeconds: 300,
  retentionDays: 7,
}

/**
 * 各字段允许区间，与后端 `RecordingConfig::normalize()` 的 clamp 范围一致。
 * 服务端同样会收敛超界值，前端提前对齐，避免保存后被静默改写。
 */
export const RECORDING_LIMITS = {
  preCaptureSeconds: { min: 5, max: 30 },
  postCaptureSeconds: { min: 5, max: 30 },
  maxFileSeconds: { min: 30, max: 3600 },
  retentionDays: { min: 1, max: 365 },
} as const

function clampToRange(value: number, range: { min: number; max: number }): number {
  if (!Number.isFinite(value)) return range.min
  return Math.min(range.max, Math.max(range.min, Math.round(value)))
}

/** 把草稿收敛到合法区间；非法数值（NaN）回落到区间下界 */
export function clampRecordingConfig(config: CameraRecordingConfig): CameraRecordingConfig {
  return {
    enabled: config.enabled,
    preCaptureSeconds: clampToRange(config.preCaptureSeconds, RECORDING_LIMITS.preCaptureSeconds),
    postCaptureSeconds: clampToRange(
      config.postCaptureSeconds,
      RECORDING_LIMITS.postCaptureSeconds,
    ),
    maxFileSeconds: clampToRange(config.maxFileSeconds, RECORDING_LIMITS.maxFileSeconds),
    retentionDays: clampToRange(config.retentionDays, RECORDING_LIMITS.retentionDays),
  }
}

/** 判断草稿相对服务端值是否有改动；服务端为 null 时以默认值为基准 */
export function isRecordingConfigDirty(
  draft: CameraRecordingConfig,
  server: CameraRecordingConfig | null,
): boolean {
  const base = server ?? DEFAULT_RECORDING_CONFIG
  return (
    draft.enabled !== base.enabled ||
    draft.preCaptureSeconds !== base.preCaptureSeconds ||
    draft.postCaptureSeconds !== base.postCaptureSeconds ||
    draft.maxFileSeconds !== base.maxFileSeconds ||
    draft.retentionDays !== base.retentionDays
  )
}
