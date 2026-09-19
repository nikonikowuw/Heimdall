import type { ReactElement } from 'react'
import { useTranslation } from 'react-i18next'

/**
 * 懒加载路由在 chunk 到达前的占位。
 *
 * `.route-fallback` 延迟 200ms 才现身：内网 chunk 通常在数十毫秒内到达，
 * 立即渲染骨架只会造成无意义的闪烁；只有慢速场景才需要让用户看到提示。
 *
 * 进度条复用已有的 `.indeterminate-track` / `.indeterminate-bar`，该组样式
 * 已自带 `prefers-reduced-motion` 降级。
 */
export function RouteFallback(): ReactElement {
  const { t } = useTranslation('common')

  return (
    <div
      role="status"
      aria-busy="true"
      className="route-fallback flex h-full w-full flex-1 items-center justify-center"
    >
      <div className="flex flex-col items-center gap-3">
        <div className="indeterminate-track h-0.5 w-24">
          <div className="indeterminate-bar" />
        </div>
        <span className="text-xs tracking-wide text-[var(--text-muted)]">{t('loading')}</span>
      </div>
    </div>
  )
}
