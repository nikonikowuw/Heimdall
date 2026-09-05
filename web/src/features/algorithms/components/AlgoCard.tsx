import React from 'react'
import { CheckCircle2, Code2, Cpu, Layers, ShieldCheck } from 'lucide-react'
import { motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgorithmItem } from '@/types'

export interface AlgoCardProps {
  algorithm: AlgorithmItem
  onManageVersions: (algo: AlgorithmItem) => void
  onViewSchema: (algo: AlgorithmItem) => void
}

export const AlgoCard: React.FC<AlgoCardProps> = ({
  algorithm,
  onManageVersions,
  onViewSchema,
}) => {
  const { t } = useTranslation('algo')

  const activeVersion =
    algorithm.versions.find((v) => v.isActive) ||
    algorithm.versions.find((v) => v.version === algorithm.activeVersion) ||
    algorithm.versions[0]

  const platformId = activeVersion ? activeVersion.platformId : 'macos-arm64'

  return (
    <motion.div
      layout
      initial={{ opacity: 0, scale: 0.98 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.98 }}
      transition={{ duration: motionTokens.duration.normal, ease: motionTokens.easing.smooth }}
      className="frosted-glass flex flex-col justify-between rounded-2xl border border-[var(--border)] p-5 transition-all duration-200 hover:border-[var(--border-strong)] hover:shadow-lg"
    >
      <div className="space-y-3.5">
        {/* 顶部标题与标签 */}
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-2.5">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
              <Cpu className="h-5 w-5" />
            </div>
            <div>
              <h3 className="text-sm font-bold text-[var(--text-primary)]">{algorithm.name}</h3>
              <span className="font-mono text-[11px] font-medium text-[var(--accent)]">
                {algorithm.algorithmId}
              </span>
            </div>
          </div>

          <div className="flex items-center gap-1.5">
            {algorithm.isBuiltin ? (
              <span className="inline-flex items-center gap-1 rounded-md border border-indigo-500/20 bg-indigo-500/10 px-2 py-0.5 text-[11px] font-semibold text-indigo-400">
                <ShieldCheck className="h-3 w-3" />
                {t('card.builtinTag')}
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 rounded-md border border-amber-500/20 bg-amber-500/10 px-2 py-0.5 text-[11px] font-semibold text-amber-400">
                {t('card.customTag')}
              </span>
            )}
          </div>
        </div>

        {/* 描述信息 */}
        <p className="line-clamp-2 text-xs leading-relaxed text-[var(--text-secondary)]">
          {algorithm.description || t('card.noDescription')}
        </p>

        {/* 运行元信息栏 */}
        <div className="flex flex-wrap items-center gap-2 pt-1 text-xs">
          <div className="inline-flex items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-1">
            <CheckCircle2 className="h-3 w-3 text-emerald-500" />
            <span className="text-[var(--text-muted)]">{t('card.activeVersion')}:</span>
            <span className="font-mono font-semibold text-[var(--text-primary)]">
              v{algorithm.activeVersion || (activeVersion ? activeVersion.version : '1.0.0')}
            </span>
          </div>

          <div className="inline-flex items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-1 font-mono text-[11px] text-[var(--text-muted)]">
            <span>{platformId}</span>
          </div>

          <div className="inline-flex items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-1 text-[11px] text-[var(--text-muted)]">
            <span>
              {algorithm.versions.length} {t('card.totalVersions')}
            </span>
          </div>
        </div>
      </div>

      {/* 底部动作操作栏 */}
      <div className="mt-4 flex items-center justify-between border-t border-[var(--border)] pt-3.5">
        <button
          type="button"
          onClick={() => onViewSchema(algorithm)}
          className="flex items-center gap-1 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:text-[var(--text-primary)]"
        >
          <Code2 className="h-3.5 w-3.5 text-[var(--accent)]" />
          <span>{t('actions.viewSchema')}</span>
        </button>

        <button
          type="button"
          onClick={() => onManageVersions(algorithm)}
          className="flex items-center gap-1 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3 py-1.5 text-xs font-medium text-[var(--text-primary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
        >
          <Layers className="h-3.5 w-3.5" />
          <span>{t('actions.manageVersions')}</span>
        </button>
      </div>
    </motion.div>
  )
}
