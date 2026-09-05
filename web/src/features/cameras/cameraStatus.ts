import type { ProbeStatus } from '@/types'

export type NormalizedProbeStatus = 'healthy' | 'degraded' | 'offline' | 'unprobed'

export interface ProbeBadgeInfo {
  status: NormalizedProbeStatus
  dotClass: string
  badgeBg: string
  text: string
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
        text: resolveText(['card.onlineStatus', 'status.healthy'], '正常在线'),
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
        text: resolveText(['card.offlineStatus', 'status.offline'], '设备离线'),
        dotClass: 'bg-rose-500',
        badgeBg: 'bg-rose-500/10 text-rose-500 border-rose-500/20',
      }
    case 'unprobed':
    default:
      return {
        status: norm,
        text: resolveText(['card.unprobedStatus', 'status.unprobed'], '待探活'),
        dotClass: 'bg-slate-400',
        badgeBg: 'bg-slate-500/10 text-slate-400 border-slate-500/20',
      }
  }
}
