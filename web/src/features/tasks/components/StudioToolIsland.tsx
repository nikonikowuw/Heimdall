import React from 'react'
import { Hexagon, Magnet, MousePointer2, Redo2, ShieldAlert, Slash, Undo2, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
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
]

/**
 * 画板左侧垂直工具岛：常驻显示，进入工作台即可直接绘制，
 * 不再需要先进入“标定模式”，消除模式切换带来的操作阻断。
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

  return (
    <div
      className="absolute top-1/2 left-3 z-30 flex -translate-y-1/2 flex-col items-center gap-1 rounded-[8px] border border-white/15 bg-[var(--video-surface)]/95 p-1.5 shadow-2xl backdrop-blur-md"
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
          <button
            key={item.id}
            type="button"
            onClick={() => onToolChange(item.id)}
            aria-pressed={isActive}
            aria-label={`${label} (${item.shortcut})`}
            title={`${label} (${item.shortcut})`}
            className={`flex h-9 w-9 items-center justify-center rounded-[6px] transition-all ${
              isActive
                ? `${item.activeClass} shadow-xs`
                : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]'
            }`}
          >
            {item.icon}
          </button>
        )
      })}

      <span className="my-0.5 h-px w-5 bg-[var(--border)]" />

      <button
        type="button"
        onClick={onToggleSnap}
        aria-pressed={snapEnabled}
        aria-label={t('tools.snap', { defaultValue: '顶点自动磁吸' })}
        title={t('studio.snapMagnet', { defaultValue: '顶点自动磁吸' })}
        className={`flex h-9 w-9 items-center justify-center rounded-[6px] transition-all ${
          snapEnabled
            ? 'text-[var(--accent)] hover:bg-[var(--accent-soft)]'
            : 'text-[var(--text-muted)] opacity-60 hover:bg-[var(--accent-soft)]'
        }`}
      >
        <Magnet className="h-4 w-4" />
      </button>

      <span className="my-0.5 h-px w-5 bg-[var(--border)]" />

      <button
        type="button"
        onClick={onUndo}
        disabled={!canUndo}
        aria-label={t('studio.undo', { defaultValue: '撤销' })}
        title={t('studio.undoHint', { defaultValue: '撤销 (Ctrl+Z)' })}
        className="flex h-9 w-9 items-center justify-center rounded-[6px] text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-35 disabled:hover:bg-transparent"
      >
        <Undo2 className="h-4 w-4" />
      </button>
      <button
        type="button"
        onClick={onRedo}
        disabled={!canRedo}
        aria-label={t('studio.redo', { defaultValue: '重做' })}
        title={t('studio.redoHint', { defaultValue: '重做 (Ctrl+Shift+Z)' })}
        className="flex h-9 w-9 items-center justify-center rounded-[6px] text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-35 disabled:hover:bg-transparent"
      >
        <Redo2 className="h-4 w-4" />
      </button>

      {isDrawing && (
        <>
          <span className="my-0.5 h-px w-5 bg-[var(--border)]" />
          <button
            type="button"
            onClick={onCancelDrawing}
            aria-label={t('studio.cancelDrawing', { defaultValue: '取消当前绘制' })}
            title={t('studio.cancelDrawing', { defaultValue: '取消当前绘制' })}
            className="flex h-9 w-9 items-center justify-center rounded-[6px] text-[var(--destructive)] transition-colors hover:bg-[var(--destructive)]/10"
          >
            <X className="h-4 w-4" />
          </button>
        </>
      )}
    </div>
  )
}
