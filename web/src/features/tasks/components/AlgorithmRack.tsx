import React from 'react'
import { Boxes, CreditCard, Flame, ScanFace, Settings2, ShieldAlert, Cpu } from 'lucide-react'
import { motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgoManifest } from '@/types'

export interface AlgorithmInstanceItem {
  algorithmId: string
  analysisFps: number
  algoParams: Record<string, unknown>
  enabled: boolean
}

export interface AlgorithmRackProps {
  availableAlgos: AlgoManifest[]
  activeInstances: Record<string, AlgorithmInstanceItem>
  onToggleAlgo: (algorithmId: string) => void
  onOpenParams: (algo: AlgoManifest) => void
}

/**
 * 根据算法类型或名称推断合适的矢量图标
 */
function getAlgoVectorIcon(algoType?: string, algoId?: string): React.ReactElement {
  const typeStr = (algoType || '').toLowerCase()
  const idStr = (algoId || '').toLowerCase()

  if (typeStr.includes('face') || idStr.includes('face')) {
    return <ScanFace className="h-5 w-5" />
  }
  if (typeStr.includes('plate') || idStr.includes('plate') || idStr.includes('lpr')) {
    return <CreditCard className="h-5 w-5" />
  }
  if (typeStr.includes('fire') || idStr.includes('fire') || idStr.includes('smoke')) {
    return <Flame className="h-5 w-5" />
  }
  if (typeStr.includes('detect') || idStr.includes('yolo') || idStr.includes('detect')) {
    return <Boxes className="h-5 w-5" />
  }
  if (typeStr.includes('behavior') || idStr.includes('intrusion') || idStr.includes('safe')) {
    return <ShieldAlert className="h-5 w-5" />
  }
  return <Cpu className="h-5 w-5" />
}

export function AlgorithmRack({
  availableAlgos,
  activeInstances,
  onToggleAlgo,
  onOpenParams,
}: AlgorithmRackProps): React.ReactElement {
  const { t } = useTranslation('task')

  // 排序：启用的排在前面，未启用的排在后面
  const sortedAlgos = React.useMemo(() => {
    return [...availableAlgos].sort((a, b) => {
      const aActive = Boolean(activeInstances[a.algorithmId]?.enabled)
      const bActive = Boolean(activeInstances[b.algorithmId]?.enabled)
      if (aActive && !bActive) return -1
      if (!aActive && bActive) return 1
      return a.name.localeCompare(b.name)
    })
  }, [availableAlgos, activeInstances])

  const activeCount = Object.values(activeInstances).filter((i) => i.enabled).length

  return (
    <div className="space-y-2.5">
      {/* 栏目标题与快速计数 */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-xs font-bold tracking-wide text-[var(--text-primary)]">
            {t('studio.algoRackTitle', { defaultValue: 'AI 智能检测引擎 (Smart Detections)' })}
          </span>
          <span className="rounded-full border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-0.5 font-mono text-[10px] font-semibold text-[var(--text-secondary)]">
            {activeCount}/{availableAlgos.length} {t('status.armed', { defaultValue: '已启用' })}
          </span>
        </div>
        <span className="font-mono text-[11px] text-[var(--text-muted)]">
          {t('studio.rackTip', { defaultValue: '点击卡片切换启用 · 启用后点齿轮调参' })}
        </span>
      </div>

      {/* 紧凑算法卡片网格：多算法并列展示，超出后在机架内部滚动 */}
      <div className="max-h-[min(42vh,360px)] overflow-y-auto pr-0.5">
        <div className="grid grid-cols-2 gap-2">
          {sortedAlgos.map((algo) => {
            const instance = activeInstances[algo.algorithmId]
            const isEnabled = Boolean(instance?.enabled)
            const icon = getAlgoVectorIcon(algo.algorithmType, algo.algorithmId)

            return (
              <motion.div
                key={algo.algorithmId}
                layout
                transition={{
                  duration: motionTokens.duration.normal,
                  ease: motionTokens.easing.smooth,
                }}
                className={`group relative flex min-h-[108px] min-w-0 flex-col rounded-[8px] border bg-[var(--bg-surface)] py-2.5 pr-2.5 pl-3.5 transition-[border-color,background-color,box-shadow] select-none ${
                  isEnabled
                    ? 'border-[var(--accent-green)]/50 bg-[var(--accent-green)]/10 shadow-[0_0_12px_rgba(16,185,129,0.08)]'
                    : 'border-[var(--border)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-secondary)]'
                }`}
              >
                {/* 启用态左侧垂直光纤指示条 (Light-pipe LED) */}
                {isEnabled && (
                  <span className="absolute top-0 bottom-0 left-0 w-1 rounded-l-[7px] bg-[var(--accent-green)] shadow-[0_0_8px_var(--accent-green)]" />
                )}

                <div className="flex min-w-0 items-start gap-2">
                  <button
                    type="button"
                    onClick={() => onToggleAlgo(algo.algorithmId)}
                    aria-pressed={isEnabled}
                    aria-label={`${algo.name} ${
                      isEnabled
                        ? t('studio.disableAlgo', { defaultValue: '已启用，点击停用' })
                        : t('studio.enableAlgo', { defaultValue: '未启用，点击启用' })
                    }`}
                    className="flex min-w-0 flex-1 items-start gap-2 text-left focus-visible:ring-2 focus-visible:ring-[var(--accent)] focus-visible:outline-none focus-visible:ring-inset"
                  >
                    <span
                      className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-[6px] border transition-colors ${
                        isEnabled
                          ? 'border-[var(--accent-green)]/30 bg-[var(--accent-green)]/15 text-[var(--accent-green)]'
                          : 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)] group-hover:text-[var(--text-secondary)]'
                      }`}
                    >
                      {icon}
                    </span>
                    <span className="min-w-0 flex-1 pt-0.5">
                      <span
                        className={`block truncate text-[11px] font-bold ${
                          isEnabled ? 'text-[var(--text-primary)]' : 'text-[var(--text-secondary)]'
                        }`}
                        title={algo.name}
                      >
                        {algo.name}
                      </span>
                      <span className="mt-1 flex items-center gap-1.5 text-[10px] text-[var(--text-muted)]">
                        <span
                          className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                            isEnabled
                              ? 'bg-[var(--accent-green)] shadow-[0_0_4px_var(--accent-green)]'
                              : 'bg-[var(--text-muted)]/50'
                          }`}
                        />
                        <span className="truncate">
                          {isEnabled
                            ? t('studio.algoEnabledStatus', { defaultValue: '已启用' })
                            : t('studio.algoDisabledStatus', { defaultValue: '未启用' })}
                        </span>
                      </span>
                    </span>
                  </button>

                  {isEnabled && (
                    <button
                      type="button"
                      onClick={() => onOpenParams(algo)}
                      title={t('studio.tuneParams', { defaultValue: '微调该算法运行参数' })}
                      aria-label={`${t('studio.tuneParams', { defaultValue: '微调该算法运行参数' })}: ${algo.name}`}
                      className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--accent)] shadow-2xs transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--accent)] focus-visible:outline-none active:scale-95"
                    >
                      <Settings2 className="h-3.5 w-3.5" />
                    </button>
                  )}
                </div>

                <div className="mt-auto flex min-w-0 items-center justify-between gap-1.5 pt-2 font-mono text-[10px]">
                  <span
                    className="min-w-0 truncate text-[var(--text-muted)]"
                    title={algo.algorithmId}
                  >
                    {algo.algorithmId}
                  </span>
                  {isEnabled && (
                    <span className="shrink-0 rounded-[4px] border border-[var(--accent-green)]/30 bg-[var(--accent-green)]/15 px-1.5 py-0.5 font-bold text-[var(--accent-green)] shadow-2xs">
                      {instance.analysisFps || 10} FPS
                    </span>
                  )}
                </div>
              </motion.div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
