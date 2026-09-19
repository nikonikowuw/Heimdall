import React from 'react'
import { CheckCircle2, Code2, Cpu, Layers, Radio, ShieldCheck, TriangleAlert } from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgorithmItem } from '@/types'
import { activeVersionItem, compatibleVersionCount, hasCompatibleVersion } from '../algoFilters'
import { blockingUsageEntries, type AlgoUsageEntry } from '../algoUsage'
import { formatBytes } from '../format'

export interface AlgoCardProps {
  algorithm: AlgorithmItem
  /** 绑定了该算法的通道任务（含未启用绑定） */
  usage: AlgoUsageEntry[]
  onManageVersions: (algo: AlgorithmItem) => void
  onViewSchema: (algo: AlgorithmItem) => void
}

const CHIP_CLASS =
  'inline-flex items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-0.5 text-[11px] text-[var(--text-muted)]'

/**
 * 算法资产卡片。
 *
 * 卡片回答三个运维问题：**当前生效的是哪个版本**、**能不能在本机跑**、**谁在用**。
 * 版本号、平台代号、体积一律用等宽数字（`font-data` + `tabular-nums`），
 * 避免多卡并排时因字形宽度差异产生抖动。
 */
export function AlgoCard({
  algorithm,
  usage,
  onManageVersions,
  onViewSchema,
}: AlgoCardProps): React.ReactElement {
  const { t } = useTranslation('algo')
  const reducedMotion = useReducedMotion()

  const activeVersion = activeVersionItem(algorithm)
  const activeVersionLabel = algorithm.activeVersion || activeVersion?.version || ''
  const compatibleCount = compatibleVersionCount(algorithm)
  const isCompatible = hasCompatibleVersion(algorithm)
  const isPartialCompatible = isCompatible && compatibleCount < algorithm.versions.length
  // 「在用」只算启用中的实例，与后端卸载保护的口径保持一致
  const blockingUsage = blockingUsageEntries(usage)
  const packageSizeBytes = algorithm.versions.reduce(
    (total, version) => total + version.packageSizeBytes,
    0,
  )

  return (
    <motion.article
      role="listitem"
      layout
      initial={reducedMotion ? false : { opacity: 0, scale: 0.98 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={reducedMotion ? undefined : { opacity: 0, scale: 0.98 }}
      transition={
        reducedMotion
          ? { duration: 0 }
          : { duration: motionTokens.duration.normal, ease: motionTokens.easing.smooth }
      }
      className="frosted-glass flex flex-col justify-between rounded-2xl border border-[var(--border)] p-5 transition-colors duration-200 hover:border-[var(--border-strong)] hover:shadow-lg"
    >
      <div className="space-y-3.5">
        <div className="flex items-start justify-between gap-3">
          <div className="flex min-w-0 items-center gap-2.5">
            <div
              className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-xl ${
                isCompatible
                  ? 'bg-[var(--accent-soft)] text-[var(--accent)]'
                  : 'bg-[var(--bg-secondary)] text-[var(--text-muted)]'
              }`}
            >
              <Cpu className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <h3 className="truncate text-sm font-bold text-[var(--text-primary)]">
                {algorithm.name}
              </h3>
              <span className="font-data block truncate text-[11px] font-medium text-[var(--accent)]">
                {algorithm.algorithmId}
              </span>
            </div>
          </div>

          {algorithm.isBuiltin ? (
            <span className="inline-flex shrink-0 items-center gap-1 rounded-md border border-indigo-500/20 bg-indigo-500/10 px-2 py-0.5 text-[11px] font-semibold text-indigo-400">
              <ShieldCheck className="h-3 w-3" />
              {t('card.builtinTag')}
            </span>
          ) : (
            <span className="inline-flex shrink-0 items-center gap-1 rounded-md border border-amber-500/20 bg-amber-500/10 px-2 py-0.5 text-[11px] font-semibold text-amber-400">
              {t('card.customTag')}
            </span>
          )}
        </div>

        <p className="line-clamp-2 text-xs leading-relaxed text-[var(--text-secondary)]">
          {algorithm.description || t('card.noDescription')}
        </p>

        {/* 运行元信息：版本 / 平台 / 版本数 / 体积压成一行，扫读靠差异项而不是字段名 */}
        <div className="font-data flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-[var(--text-muted)] tabular-nums">
          <span className="inline-flex items-center gap-1">
            <CheckCircle2 className="h-3 w-3 text-emerald-500" />
            <span className="font-semibold text-[var(--text-primary)]">
              v{activeVersionLabel || t('card.versionUnknown')}
            </span>
          </span>
          {activeVersion && (
            <>
              <span aria-hidden="true">·</span>
              <span>{activeVersion.normalizedPlatformId}</span>
            </>
          )}
          <span aria-hidden="true">·</span>
          <span>{t('card.versionCount', { count: algorithm.versions.length })}</span>
          {packageSizeBytes > 0 && (
            <>
              <span aria-hidden="true">·</span>
              <span>{formatBytes(packageSizeBytes)}</span>
            </>
          )}
        </div>

        {/* 兼容性与占用的显式警示：不靠图标颜色单独传意，一律带文字 */}
        {(!isCompatible || isPartialCompatible || blockingUsage.length > 0) && (
          <div className="flex flex-wrap items-center gap-2">
            {!isCompatible && (
              <span className="inline-flex items-center gap-1 rounded-lg border border-amber-500/20 bg-amber-500/10 px-2 py-0.5 text-[11px] font-semibold text-amber-400">
                <TriangleAlert className="h-3 w-3" />
                {t('card.incompatible')}
              </span>
            )}
            {isPartialCompatible && (
              <span className={CHIP_CLASS}>
                {t('card.compatibleVersions', {
                  compatible: compatibleCount,
                  total: algorithm.versions.length,
                })}
              </span>
            )}
            {blockingUsage.length > 0 && (
              <span
                className={CHIP_CLASS}
                title={blockingUsage
                  .map((entry) => `${entry.taskName} (${entry.cameraId})`)
                  .join('\n')}
              >
                <Radio className="h-3 w-3 text-[var(--accent)]" />
                {t('card.inUseBy', { count: blockingUsage.length })}
              </span>
            )}
          </div>
        )}
      </div>

      <div className="mt-4 flex items-center justify-between border-t border-[var(--border)] pt-3.5">
        <button
          type="button"
          onClick={() => onViewSchema(algorithm)}
          className="flex items-center gap-1 rounded-lg px-1 py-1 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
        >
          <Code2 className="h-3.5 w-3.5 text-[var(--accent)]" />
          <span>{t('actions.viewSchema')}</span>
        </button>

        <button
          type="button"
          onClick={() => onManageVersions(algorithm)}
          className="flex items-center gap-1 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3 py-1.5 text-xs font-medium text-[var(--text-primary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden active:scale-95"
        >
          <Layers className="h-3.5 w-3.5" />
          <span>{t('actions.manageVersions')}</span>
        </button>
      </div>
    </motion.article>
  )
}
