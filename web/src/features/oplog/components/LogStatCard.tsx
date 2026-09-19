import type { ReactElement, ReactNode } from 'react'
import type { LucideIcon } from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { getToneClasses, type LogTone } from '../logTone'

interface LogStatCardProps {
  /** 指标名称 */
  label: string
  /** 主数值 */
  value: ReactNode
  icon: LucideIcon
  /** 图标底托色调 */
  tone: LogTone
  /** 数值右侧的补充说明，例如占比、分级标签 */
  detail?: ReactNode
  /** 主数值强调色；不传时使用主文本色 */
  valueTone?: LogTone
  /** 传入后卡片变为可点击的筛选快捷入口 */
  onFilterToggle?: () => void
  /** 当前是否处于该卡片的筛选态 */
  isFilterActive?: boolean
  /** 入场动画序号，用于错峰淡入 */
  index?: number
}

/**
 * 日志指标卡外壳：两个日志域的统计卡片共用同一份版式与动效，
 * 各自只负责计算指标与文案，避免卡片结构在多个文件里重复维护。
 */
export function LogStatCard({
  label,
  value,
  icon: Icon,
  tone,
  detail,
  valueTone,
  onFilterToggle,
  isFilterActive = false,
  index = 0,
}: LogStatCardProps): ReactElement {
  const reducedMotion = useReducedMotion()
  const toneClasses = getToneClasses(tone)

  const baseClass =
    'frosted-glass relative flex flex-col justify-between overflow-hidden rounded-xl p-3 text-left shadow-xs'
  const motionProps = {
    initial: reducedMotion ? false : { opacity: 0, y: 6 },
    animate: { opacity: 1, y: 0 },
    transition: { duration: 0.2, delay: index * 0.04 },
  }

  const body = (
    <>
      <div className="flex items-center justify-between gap-2">
        <span className="text-[11px] font-medium text-[var(--text-muted)]">{label}</span>
        <div className={`flex h-7 w-7 items-center justify-center rounded-lg ${toneClasses.chip}`}>
          <Icon className="h-3.5 w-3.5" />
        </div>
      </div>
      <div className="mt-2 flex items-baseline gap-2">
        <span
          className={`font-tech text-xl font-bold tracking-tight ${
            valueTone ? getToneClasses(valueTone).text : 'text-[var(--text-primary)]'
          }`}
        >
          {value}
        </span>
        {detail ? <span className="text-[10px] text-[var(--text-muted)]">{detail}</span> : null}
      </div>
    </>
  )

  if (onFilterToggle) {
    return (
      <motion.button
        type="button"
        onClick={onFilterToggle}
        aria-pressed={isFilterActive}
        {...motionProps}
        className={`${baseClass} transition-all ${
          isFilterActive
            ? 'border-[var(--accent)] ring-2 ring-[var(--accent)]/30'
            : 'hover:border-[var(--border-strong)]'
        }`}
      >
        {body}
      </motion.button>
    )
  }

  return (
    <motion.div {...motionProps} className={baseClass}>
      {body}
    </motion.div>
  )
}
