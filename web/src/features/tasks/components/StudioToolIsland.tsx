import React from 'react'
import {
  Crop,
  Hexagon,
  Magnet,
  MousePointer2,
  Redo2,
  ShieldAlert,
  Slash,
  Undo2,
  X,
} from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import type { ToolMode } from './rulesStudioTypes'

export interface StudioToolIslandProps {
  tool: ToolMode
  onToolChange: (tool: ToolMode) => void
  snapEnabled: boolean
  onToggleSnap: () => void
  canUndo: boolean
  canRedo: boolean
  onUndo: () => void
  onRedo: () => void
  /** 正在绘制未完成的图形（用于显示取消入口） */
  isDrawing: boolean
  onCancelDrawing: () => void
}

interface ToolDefinition {
  id: ToolMode
  icon: React.ReactNode
  /** i18n key */
  labelKey: string
  shortcut: string
  /** 激活态强调色类名 */
  activeClass: string
}

const TOOLS: ToolDefinition[] = [
  {
    id: 'select',
    icon: <MousePointer2 className="h-4 w-4" />,
    labelKey: 'tools.select',
    shortcut: 'V',
    activeClass: 'bg-[var(--accent)] text-white',
  },
  {
    id: 'roi',
    icon: <Hexagon className="h-4 w-4" />,
    labelKey: 'tools.roi',
    shortcut: 'P',
    activeClass: 'bg-[var(--accent)] text-white',
  },
  {
    id: 'line',
    icon: <Slash className="h-4 w-4" />,
    labelKey: 'tools.line',
    shortcut: 'L',
    activeClass: 'bg-[var(--accent)] text-white',
  },
  {
    id: 'mask',
    icon: <ShieldAlert className="h-4 w-4" />,
    labelKey: 'tools.mask',
    shortcut: 'M',
    activeClass: 'bg-[var(--accent)] text-white',
  },
  {
    id: 'precrop',
    icon: <Crop className="h-4 w-4" />,
    labelKey: 'tools.precrop',
    shortcut: 'C',
    activeClass: 'bg-[var(--accent)] text-white',
  },
]

/**
 * 画板左侧垂直工具岛：常驻显示，进入工作台即可直接绘制，
 * 采用 VisionOS 极客深色磨砂与按键微光设计。
 */
export function StudioToolIsland({
  tool,
  onToolChange,
  snapEnabled,
  onToggleSnap,
  canUndo,
  canRedo,
  onUndo,
  onRedo,
  isDrawing,
  onCancelDrawing,
}: StudioToolIslandProps): React.ReactElement {
  const { t } = useTranslation('task')
  const reduceMotion = useReducedMotion()

  return (
    <motion.div
      initial={{ opacity: 0, x: reduceMotion ? 0 : -motionTokens.distance.md }}
      animate={{ opacity: 1, x: 0 }}
      transition={{
        duration: reduceMotion ? 0 : motionTokens.duration.normal,
        ease: motionTokens.easing.smooth,
      }}
      className="absolute top-1/2 left-3 z-30 flex -translate-y-1/2 flex-col items-center gap-1.5 rounded-xl border border-white/15 bg-black/65 p-1.5 shadow-[0_16px_48px_rgba(0,0,0,0.7)] backdrop-blur-2xl"
      onMouseDown={(event) => event.stopPropagation()}
      onMouseMove={(event) => event.stopPropagation()}
      onClick={(event) => event.stopPropagation()}
      onDoubleClick={(event) => event.stopPropagation()}
      role="toolbar"
      aria-orientation="vertical"
      aria-label={t('studio.toolbarLabel', { defaultValue: '防区绘制工具' })}
    >
      {TOOLS.map((item) => {
        const isActive = tool === item.id
        const label = t(item.labelKey, { defaultValue: item.id })
        return (
          <motion.button
            key={item.id}
            type="button"
            onClick={() => onToolChange(item.id)}
            whileHover={reduceMotion ? undefined : { scale: 1.05 }}
            whileTap={reduceMotion ? undefined : { scale: 0.94 }}
            aria-pressed={isActive}
            aria-label={`${label} (${item.shortcut})`}
            title={`${label} (${item.shortcut})`}
            className={`group relative flex h-9 w-9 items-center justify-center rounded-lg transition-colors duration-150 ${
              isActive ? 'text-white' : 'text-white/70 hover:bg-white/10 hover:text-white'
            }`}
          >
            {isActive && (
              <motion.span
                layoutId="activeToolBubble"
                transition={{
                  duration: reduceMotion ? 0 : motionTokens.duration.fast,
                  ease: motionTokens.easing.smooth,
                }}
                className="absolute inset-0 rounded-lg bg-[var(--accent)] shadow-[0_0_14px_rgba(59,130,246,0.55)] ring-1 ring-white/30"
              />
            )}
            <span className="relative z-10">{item.icon}</span>
            <span className="pointer-events-none absolute right-0.5 bottom-0.5 z-10 font-mono text-[8px] font-bold opacity-60 select-none group-hover:opacity-90">
              {item.shortcut}
            </span>
          </motion.button>
        )
      })}

      <span className="my-0.5 h-px w-5 bg-white/15" />

      {/* 顶点自动磁吸 */}
      <motion.button
        type="button"
        onClick={onToggleSnap}
        whileHover={reduceMotion ? undefined : { scale: 1.05 }}
        whileTap={reduceMotion ? undefined : { scale: 0.94 }}
        aria-pressed={snapEnabled}
        aria-label={t('tools.snap', { defaultValue: '顶点自动磁吸' })}
        title={t('studio.snapMagnet', { defaultValue: '顶点自动磁吸 (S)' })}
        className={`group relative flex h-9 w-9 items-center justify-center rounded-lg transition-all duration-150 ${
          snapEnabled
            ? 'bg-emerald-500/20 text-emerald-400 ring-1 ring-emerald-500/40 hover:bg-emerald-500/25'
            : 'text-white/40 hover:bg-white/10 hover:text-white/70'
        }`}
      >
        <Magnet className="h-4 w-4" />
        {snapEnabled && (
          <span className="absolute top-1 right-1 h-1.5 w-1.5 rounded-full bg-emerald-400 shadow-[0_0_6px_#34d399]" />
        )}
        <span className="pointer-events-none absolute right-0.5 bottom-0.5 font-mono text-[8px] font-bold opacity-60 select-none group-hover:opacity-90">
          S
        </span>
      </motion.button>

      <span className="my-0.5 h-px w-5 bg-white/15" />

      {/* 撤销 / 重做 */}
      <motion.button
        type="button"
        onClick={onUndo}
        disabled={!canUndo}
        whileHover={reduceMotion || !canUndo ? undefined : { scale: 1.05 }}
        whileTap={reduceMotion || !canUndo ? undefined : { scale: 0.94 }}
        aria-label={t('studio.undo', { defaultValue: '撤销' })}
        title={t('studio.undoHint', { defaultValue: '撤销 (Ctrl+Z)' })}
        className="flex h-9 w-9 items-center justify-center rounded-lg text-white/70 transition-all hover:bg-white/10 hover:text-white disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:bg-transparent"
      >
        <Undo2 className="h-4 w-4" />
      </motion.button>
      <motion.button
        type="button"
        onClick={onRedo}
        disabled={!canRedo}
        whileHover={reduceMotion || !canRedo ? undefined : { scale: 1.05 }}
        whileTap={reduceMotion || !canRedo ? undefined : { scale: 0.94 }}
        aria-label={t('studio.redo', { defaultValue: '重做' })}
        title={t('studio.redoHint', { defaultValue: '重做 (Ctrl+Shift+Z)' })}
        className="flex h-9 w-9 items-center justify-center rounded-lg text-white/70 transition-all hover:bg-white/10 hover:text-white disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:bg-transparent"
      >
        <Redo2 className="h-4 w-4" />
      </motion.button>

      <AnimatePresence mode="popLayout">
        {isDrawing && (
          <motion.div
            key="cancel-drawing-button"
            initial={{ opacity: 0, scale: 0.8 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.8 }}
            transition={{
              duration: reduceMotion ? 0 : motionTokens.duration.fast,
              ease: motionTokens.easing.smooth,
            }}
            className="flex flex-col items-center gap-1.5"
          >
            <span className="my-0.5 h-px w-5 bg-white/15" />
            <motion.button
              type="button"
              onClick={onCancelDrawing}
              whileHover={reduceMotion ? undefined : { scale: 1.08 }}
              whileTap={reduceMotion ? undefined : { scale: 0.92 }}
              aria-label={t('studio.cancelDrawing', { defaultValue: '取消当前绘制' })}
              title={t('studio.cancelDrawing', { defaultValue: '取消当前绘制 (Esc)' })}
              className="flex h-9 w-9 animate-pulse items-center justify-center rounded-lg bg-[var(--destructive)]/20 text-[var(--destructive)] ring-1 ring-[var(--destructive)]/40 transition-colors hover:bg-[var(--destructive)]/30"
            >
              <X className="h-4 w-4" />
            </motion.button>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  )
}
