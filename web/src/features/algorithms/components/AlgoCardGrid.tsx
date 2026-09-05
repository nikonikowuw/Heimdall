import React from 'react'
import { Cpu } from 'lucide-react'
import { AnimatePresence } from 'motion/react'
import { useTranslation } from 'react-i18next'
import type { AlgorithmItem } from '@/types'
import { AlgoCard } from './AlgoCard'

export interface AlgoCardGridProps {
  algorithms: AlgorithmItem[]
  isLoading?: boolean
  onManageVersions: (algo: AlgorithmItem) => void
  onViewSchema: (algo: AlgorithmItem) => void
}

export const AlgoCardGrid: React.FC<AlgoCardGridProps> = ({
  algorithms,
  isLoading,
  onManageVersions,
  onViewSchema,
}) => {
  const { t } = useTranslation('algo')

  if (isLoading && algorithms.length === 0) {
    return (
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
        {[1, 2, 3].map((n) => (
          <div
            key={n}
            className="frosted-glass h-48 animate-pulse rounded-2xl border border-[var(--border)] p-5"
          />
        ))}
      </div>
    )
  }

  if (algorithms.length === 0) {
    return (
      <div className="frosted-glass flex flex-col items-center justify-center rounded-2xl border border-[var(--border)] py-16 text-center">
        <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-[var(--accent-soft)] text-[var(--accent)]">
          <Cpu className="h-6 w-6" />
        </div>
        <h3 className="mt-4 text-sm font-semibold text-[var(--text-primary)]">
          {t('filter.searchPlaceholder')}
        </h3>
        <p className="mt-1 text-xs text-[var(--text-muted)]">{t('card.noDescription')}</p>
      </div>
    )
  }

  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
      <AnimatePresence mode="popLayout">
        {algorithms.map((algo) => (
          <AlgoCard
            key={algo.algorithmId}
            algorithm={algo}
            onManageVersions={onManageVersions}
            onViewSchema={onViewSchema}
          />
        ))}
      </AnimatePresence>
    </div>
  )
}
