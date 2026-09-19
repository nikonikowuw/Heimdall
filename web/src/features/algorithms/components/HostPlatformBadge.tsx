import React from 'react'
import { Server } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { HostPlatformInfo } from '@/types'
import { describeHostAccelerator } from '../algoPlatform'

export interface HostPlatformBadgeProps {
  platform: HostPlatformInfo | null
}

/**
 * 宿主推理平台标牌。
 *
 * 平台代号来自后端（编译期目标平台 + 后端 feature），不再用 `navigator.platform` 推断：
 * 后者描述的是浏览器所在机器，通过局域网远程打开控制台时会给出完全错误的宿主信息。
 */
export function HostPlatformBadge({ platform }: HostPlatformBadgeProps): React.ReactElement {
  const { t } = useTranslation('algo')

  const normalizedId = platform?.normalizedPlatformId ?? ''
  const accelerator = describeHostAccelerator(normalizedId)

  return (
    <div
      className="frosted-glass flex items-center gap-2.5 rounded-2xl border border-[var(--border)] py-2 pr-3.5 pl-2.5"
      title={platform?.platformId ?? undefined}
    >
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
        <Server className="h-4 w-4" />
      </div>
      <div className="leading-tight">
        <div className="text-[11px] font-medium text-[var(--text-muted)]">
          {t('stats.platformArch')}
        </div>
        <div className="mt-0.5 flex items-baseline gap-1.5">
          <span className="font-data text-xs font-semibold text-[var(--text-primary)] tabular-nums">
            {normalizedId || '—'}
          </span>
          {accelerator && normalizedId && (
            <span className="text-[11px] text-[var(--text-muted)]">{accelerator}</span>
          )}
        </div>
      </div>
    </div>
  )
}
