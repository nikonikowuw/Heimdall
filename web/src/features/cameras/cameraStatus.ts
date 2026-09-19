import type { ProbeStatus, TaskSummaryDto } from '@/types'

export type NormalizedProbeStatus = 'healthy' | 'degraded' | 'offline' | 'unprobed'
export type CameraAiRuntimeStatus = 'active' | 'starting' | 'degraded' | 'error' | 'inactive'

export type CameraTaskRuntimeView = Pick<
  TaskSummaryDto,
  | 'desiredEnabled'
  | 'actualStatus'
  | 'algorithmId'
  | 'analysisFps'
  | 'algorithmInstanceCount'
  | 'algorithmInstances'
  | 'updatedAt'
  | 'statusMessage'
>

export interface ProbeBadgeInfo {
  status: NormalizedProbeStatus
  dotClass: string
  badgeBg: string
  text: string
}

export interface CameraAiRuntimeSummary {
  status: CameraAiRuntimeStatus
  isActive: boolean
  activeAlgorithmIds: string[]
}

export function normalizeProbeStatus(status?: string | ProbeStatus | null): NormalizedProbeStatus {
  switch (status?.toLowerCase()) {
    case 'healthy':
    case 'success':
    case 'online':
      return 'healthy'
    case 'degraded':
    case 'reconnecting':
      return 'degraded'
    case 'failed':
    case 'offline':
    case 'error':
      return 'offline'
    default:
      return 'unprobed'
  }
}

export function getActiveAlgorithmIds(task?: CameraTaskRuntimeView | null): string[] {
  if (!task || !task.desiredEnabled) return []

  const enabledIds = (task.algorithmInstances ?? [])
    .filter((instance) => instance.enabled)
    .map((instance) => instance.algorithmId.trim())
    .filter(Boolean)

  if (enabledIds.length > 0) {
    return [...new Set(enabledIds)]
  }

  const fallbackId = task.algorithmId?.trim()
  return fallbackId ? [fallbackId] : []
}

export function getCameraAiRuntimeStatus(
  task?: CameraTaskRuntimeView | null,
): CameraAiRuntimeSummary {
  const activeAlgorithmIds = getActiveAlgorithmIds(task)
  const hasConfiguredAlgorithms = Boolean(
    task &&
    ((task.algorithmInstances?.length ?? 0) > 0 ||
      (task.algorithmInstanceCount ?? 0) > 0 ||
      task.algorithmId?.trim()),
  )

  if (!task || !task.desiredEnabled || !hasConfiguredAlgorithms) {
    return { status: 'inactive', isActive: false, activeAlgorithmIds }
  }

  switch (task.actualStatus) {
    case 2:
      return { status: 'active', isActive: true, activeAlgorithmIds }
    case 1:
      return { status: 'starting', isActive: false, activeAlgorithmIds }
    case 3:
    case 4:
      return { status: 'degraded', isActive: false, activeAlgorithmIds }
    case 5:
      return { status: 'error', isActive: false, activeAlgorithmIds }
    default:
      return { status: 'inactive', isActive: false, activeAlgorithmIds }
  }
}

export function getProbeBadge(
  status?: string | ProbeStatus | null,
  customTextOrT?: string | ((key: string) => string),
): ProbeBadgeInfo {
  const norm = normalizeProbeStatus(status)

  const resolveText = (keys: string[], defaultText: string): string => {
    if (!customTextOrT) return defaultText
    if (typeof customTextOrT === 'string') return customTextOrT
    for (const k of keys) {
      const val = customTextOrT(k)
      if (val && val !== k) return val
    }
    return defaultText
  }

  switch (norm) {
    case 'healthy':
      return {
        status: norm,
        text: resolveText(['card.onlineStatus', 'status.online', 'status.healthy'], '在线'),
        dotClass: 'bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.6)] animate-pulse',
        badgeBg: 'bg-emerald-500/10 text-emerald-500 border-emerald-500/20',
      }
    case 'degraded':
      return {
        status: norm,
        text: resolveText(['card.degradedStatus', 'status.degraded'], '网络波动'),
        dotClass: 'bg-amber-500 shadow-[0_0_6px_rgba(245,158,11,0.5)]',
        badgeBg: 'bg-amber-500/10 text-amber-500 border-amber-500/20',
      }
    case 'offline':
      return {
        status: norm,
        text: resolveText(['card.offlineStatus', 'status.offline'], '离线'),
        dotClass: 'bg-rose-500',
        badgeBg: 'bg-rose-500/10 text-rose-500 border-rose-500/20',
      }
    case 'unprobed':
    default:
      return {
        status: norm,
        text: resolveText(['card.unprobedStatus', 'status.unprobed'], '待探测'),
        dotClass: 'bg-slate-400',
        badgeBg: 'bg-slate-500/10 text-slate-400 border-slate-500/20',
      }
  }
}

/** 统一解析探活按钮的文案状态（探活中 / 成功 / 失败 / 默认文案） */
export function getProbeButtonText(
  isProbing: boolean,
  probeFeedback: 'success' | 'failed' | undefined,
  t: (key: string, options?: { defaultValue?: string }) => string,
  defaultActionKey = 'manage.probeAction',
  defaultActionText = '探活',
): string {
  if (isProbing) {
    return t('manage.probing', { defaultValue: '探活中...' })
  }
  if (probeFeedback === 'success') {
    return t('manage.probeSuccess', { defaultValue: '探活成功' })
  }
  if (probeFeedback === 'failed') {
    return t('manage.probeFailed', { defaultValue: '探活失败' })
  }
  return t(defaultActionKey, { defaultValue: defaultActionText })
}
