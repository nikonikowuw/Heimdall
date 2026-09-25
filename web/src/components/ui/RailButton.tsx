import type { ReactElement, ReactNode } from 'react'
import { cn } from '@/lib/utils'

export interface RailButtonProps {
  icon: ReactNode
  label: string
  onClick: () => void
  /** 当前视图高亮，需要配合 activeIndicator 使用 */
  active?: boolean
  'aria-label'?: string
  'aria-expanded'?: boolean
  'aria-haspopup'?: 'menu' | 'listbox'
  /** 选中指示层，由调用方注入以复用 NavActiveIndicator 的共享 layout 动画 */
  activeIndicator?: ReactNode
}

/**
 * 侧边栏统一的图标入口按钮（等宽图标格，本项目侧栏恒为固定窄轨）。
 *
 * 标签同时以两种形式存在，各司其职：
 * - `.nav-tooltip`：纯 CSS 浮层，无延迟且可随键盘焦点触发（原生 title 两者都做不到）；
 * - `sr-only`：稳定的可访问名，读屏不依赖浮层。
 */
export function RailButton({
  icon,
  label,
  onClick,
  active = false,
  activeIndicator,
  ...aria
}: RailButtonProps): ReactElement {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? 'page' : undefined}
      {...aria}
      className={cn(
        'nav-btn',
        active ? 'text-white' : 'text-[var(--text-secondary)] hover:text-[var(--accent)]',
      )}
    >
      {activeIndicator}
      <span className="relative z-10 flex shrink-0 items-center justify-center">{icon}</span>
      {/* 纯视觉浮层：对读屏隐藏，可访问名由下行的 sr-only 承担，避免重复朗读 */}
      <span className="nav-tooltip" aria-hidden="true">
        {label}
      </span>
      <span className="sr-only">{label}</span>
    </button>
  )
}
