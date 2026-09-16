import React from 'react'
import { Minimize2, Star } from 'lucide-react'
import type { EvidenceImageSource, EvidenceImageStream } from '../../../types'
import { type EvidenceOriginBadgeKind, deriveEvidenceOriginBadges } from '../utils'

export interface EvidenceOriginBadgeProps {
  imageSource?: EvidenceImageSource | null
  imageStream?: EvidenceImageStream | null
  t: (key: string) => string
  className?: string
}

const BADGE_CONFIG = {
  peakFrame: {
    Icon: Star,
    labelKey: 'card.peakFrame',
    hintKey: 'card.peakFrameHint',
    className: 'border-cyan-500/40 bg-cyan-500/15 text-cyan-400',
  },
  subStream: {
    Icon: Minimize2,
    labelKey: 'card.subStream',
    hintKey: 'card.subStreamHint',
    className: 'border-[var(--border)] bg-black/60 text-[var(--text-secondary)]',
  },
} as const satisfies Record<EvidenceOriginBadgeKind, unknown>

/**
 * 证据图来源徽标：标明凭据取自峰值候选帧还是触发当帧、以及分辨率来自哪条码流。
 *
 * 展示决策由 `deriveEvidenceOriginBadges` 统一裁决（历史记录与未知取值一律不渲染）。
 */
export function EvidenceOriginBadge({
  imageSource,
  imageStream,
  t,
  className = '',
}: EvidenceOriginBadgeProps): React.ReactElement | null {
  const badges = deriveEvidenceOriginBadges(imageSource, imageStream)
  if (badges.length === 0) {
    return null
  }

  return (
    <div className={`flex items-center gap-1 ${className}`}>
      {badges.map((kind) => {
        const { Icon, labelKey, hintKey, className: badgeClass } = BADGE_CONFIG[kind]
        return (
          <span
            key={kind}
            title={t(hintKey)}
            className={`inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 font-mono text-[10px] backdrop-blur-md ${badgeClass}`}
          >
            <Icon className="h-2.5 w-2.5" />
            {t(labelKey)}
          </span>
        )
      })}
    </div>
  )
}
