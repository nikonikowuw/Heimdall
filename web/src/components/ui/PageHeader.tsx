import { isValidElement, type ReactElement, type ReactNode } from 'react'
import type { LucideIcon } from 'lucide-react'
import { cn } from '@/lib/utils'

export interface PageHeaderProps {
  /** 页面主图标：Lucide 组件类，或自定义 ReactNode */
  icon?: LucideIcon | ReactNode
  /** 图标外壳的覆盖样式（如按状态变色）。默认走 accent 基调 */
  iconClassName?: string
  /** 叠加在图标右下角的状态指示标（如实时呼吸灯） */
  iconIndicator?: ReactNode
  /** 页面主标题 */
  title: ReactNode
  /** 紧随标题右侧的徽章或状态标签（如 WebCodecs、硬解、总数） */
  badges?: ReactNode
  /** 标题下方的副标题文字或遥测指标行 */
  subtitle?: ReactNode
  /** 右侧操作插槽：按钮组、二级药丸 Tab、模式切换器等 */
  actions?: ReactNode
  /** 外层容器的自定义类名 */
  className?: string
}

/**
 * Lucide 图标本身是函数组件；经 memo / forwardRef 包装后变成带 render 的对象。
 * 已是 React 元素（调用方直接传 <Xxx />）时不算组件，按原样渲染。
 */
function isIconComponent(icon: LucideIcon | ReactNode): icon is LucideIcon {
  if (typeof icon === 'function') return true
  if (isValidElement(icon)) return false
  return typeof icon === 'object' && icon !== null && 'render' in icon
}

/** 图标外壳内的字形：组件类统一注入 h-5 w-5，自带尺寸的 ReactNode 原样渲染 */
function IconGlyph({ icon }: { icon: LucideIcon | ReactNode }): ReactElement {
  if (!isIconComponent(icon)) return <>{icon}</>
  const Glyph = icon
  return <Glyph className="h-5 w-5" />
}

/**
 * 全站统一的 SaaS / HUD 页面顶部标题栏规范组件。
 *
 * 几何契约（所有页面共用同一份基准，切换 Tab 时不得发生位移或高度跳变）：
 * - 高度固定 `min-h-[68px] sm:h-[68px]`，与是否提供 subtitle 无关；
 * - 内边距 `px-4 py-2.5`；图标外壳恒定 `h-10 w-10`，内部图标 `h-5 w-5`；
 * - 右侧操作区禁止换行，避免窄视口下换行撑高标题栏。
 */
export function PageHeader({
  icon,
  iconClassName,
  iconIndicator,
  title,
  badges,
  subtitle,
  actions,
  className,
}: PageHeaderProps): ReactElement {
  return (
    <header
      className={cn(
        'frosted-glass flex min-h-[68px] shrink-0 items-center justify-between gap-3 rounded-2xl px-4 py-2.5 shadow-xs select-none sm:h-[68px]',
        className,
      )}
    >
      <div className="flex min-w-0 items-center gap-3.5">
        {icon && (
          <div
            className={cn(
              'relative flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-[var(--accent)]/15 bg-[var(--accent-soft)] text-[var(--accent)] shadow-2xs',
              iconClassName,
            )}
          >
            <IconGlyph icon={icon} />
            {iconIndicator}
          </div>
        )}
        <div className="flex min-w-0 flex-col justify-center">
          <div className="flex min-w-0 items-center gap-2">
            <h1 className="truncate text-sm leading-snug font-bold tracking-tight text-[var(--text-primary)] sm:text-base">
              {title}
            </h1>
            {badges && <div className="flex shrink-0 items-center gap-1.5">{badges}</div>}
          </div>
          {subtitle && (
            <div className="mt-0.5 truncate text-[11px] leading-tight text-[var(--text-muted)] sm:text-xs">
              {subtitle}
            </div>
          )}
        </div>
      </div>

      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </header>
  )
}
