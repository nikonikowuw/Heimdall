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
          {t('studio.rackTip', { defaultValue: '点击方块直接启闭 · 启用后点齿轮调参' })}
        </span>
      </div>

      {/* 小方形卡片网格机架 (自适应 3 列排列) */}
      <div className="grid grid-cols-3 gap-2.5">
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
              onClick={() => onToggleAlgo(algo.algorithmId)}
              className={`group relative flex h-24 w-full cursor-pointer flex-col justify-between rounded-2xl border p-2.5 transition-all select-none ${
                isEnabled
                  ? 'border-[var(--accent)] bg-[var(--accent-soft)]/50 shadow-xs ring-1 ring-[var(--accent)]/30'
                  : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-muted)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-secondary)]'
              }`}
            >
              {/* 卡片顶行：状态指示灯 + 调参齿轮按钮 */}
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5">
                  <span
                    className={`h-2 w-2 rounded-full ${
                      isEnabled
                        ? 'animate-pulse bg-emerald-400 shadow-[0_0_8px_rgba(52,211,153,0.8)]'
                        : 'bg-zinc-600'
                    }`}
                  />
                  {isEnabled && (
                    <span className="font-mono text-[9px] font-bold text-emerald-400">
                      {instance.analysisFps || 10}F
                    </span>
                  )}
                </div>

                {/* 调参按钮：仅在启用时浮现 */}
                {isEnabled && (
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation()
                      onOpenParams(algo)
                    }}
                    title={t('studio.tuneParams', { defaultValue: '微调该算法运行参数' })}
                    className="flex h-6 w-6 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--accent)] transition-all hover:scale-110 hover:bg-[var(--accent)] hover:text-white"
                  >
                    <Settings2 className="h-3.5 w-3.5" />
                  </button>
                )}
              </div>

              {/* 卡片中心：矢量图标 */}
              <div
                className={`flex items-center justify-center transition-colors ${
                  isEnabled
                    ? 'text-[var(--accent)]'
                    : 'text-[var(--text-muted)] group-hover:text-[var(--text-secondary)]'
                }`}
              >
                {icon}
              </div>

              {/* 卡片底行：算法名称 */}
              <div className="truncate text-center">
                <span
                  className={`truncate text-[11px] font-semibold ${
                    isEnabled ? 'text-[var(--text-primary)]' : 'text-[var(--text-muted)]'
                  }`}
                  title={algo.name}
                >
                  {algo.name}
                </span>
              </div>
            </motion.div>
          )
        })}
      </div>
    </div>
  )
}
