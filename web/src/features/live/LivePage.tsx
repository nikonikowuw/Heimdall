import React, { useState } from 'react'
import { Camera, LayoutGrid, Maximize2, ShieldAlert } from 'lucide-react'
import { useTranslation } from 'react-i18next'

export const LivePage: React.FC = () => {
  const { t } = useTranslation('camera')
  const [gridSize, setGridSize] = useState<1 | 4 | 9>(4)

  return (
    <div className="flex h-full flex-col gap-4">
      {/* 顶部监控工具栏 */}
      <div className="frosted-glass flex items-center justify-between rounded-xl p-3">
        <div className="flex items-center gap-2">
          <Camera className="h-5 w-5 text-[var(--accent)]" />
          <span className="font-semibold text-[var(--text-primary)]">{t('live.title')}</span>
          <span className="rounded-full bg-emerald-500/10 px-2 py-0.5 text-xs text-emerald-600 dark:text-emerald-400">
            {t('live.protocolBadge')}
          </span>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={() => setGridSize(1)}
            className={`rounded-lg px-2.5 py-1 text-xs font-medium transition-colors ${
              gridSize === 1
                ? 'bg-[var(--accent)] text-white'
                : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]'
            }`}
          >
            {t('live.single')}
          </button>
          <button
            onClick={() => setGridSize(4)}
            className={`rounded-lg px-2.5 py-1 text-xs font-medium transition-colors ${
              gridSize === 4
                ? 'bg-[var(--accent)] text-white'
                : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]'
            }`}
          >
            {t('live.quad')}
          </button>
          <button
            onClick={() => setGridSize(9)}
            className={`rounded-lg px-2.5 py-1 text-xs font-medium transition-colors ${
              gridSize === 9
                ? 'bg-[var(--accent)] text-white'
                : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]'
            }`}
          >
            {t('live.nine')}
          </button>
        </div>
      </div>

      {/* 监控视频流视口网格 */}
      <div
        className={`grid flex-1 gap-3 overflow-hidden ${
          gridSize === 1
            ? 'grid-cols-1 grid-rows-1'
            : gridSize === 4
              ? 'grid-cols-2 grid-rows-2'
              : 'grid-cols-3 grid-rows-3'
        }`}
      >
        {Array.from({ length: gridSize }).map((_, idx) => (
          <div
            key={idx}
            className="group relative flex flex-col justify-between overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]"
          >
            {/* 顶层透明 Canvas 识别框图层 */}
            <canvas className="pointer-events-none absolute inset-0 z-10 h-full w-full" />

            {/* 视频占位/原生视频播放器 */}
            <div className="flex flex-1 items-center justify-center text-[var(--text-muted)]">
              <div className="flex flex-col items-center gap-2">
                <LayoutGrid className="h-8 w-8 opacity-40" />
                <span className="text-xs">{t('live.pending', { id: idx + 1 })}</span>
              </div>
            </div>

            {/* 底部信息条 */}
            <div className="frosted-glass absolute right-2 bottom-2 left-2 z-20 flex items-center justify-between rounded-lg px-3 py-1.5 opacity-80 backdrop-blur-md transition-opacity group-hover:opacity-100">
              <div className="flex items-center gap-2">
                <span className="h-2 w-2 animate-pulse rounded-full bg-emerald-500" />
                <span className="font-mono text-xs text-[var(--text-primary)]">
                  CAM-{String(idx + 1).padStart(2, '0')}
                </span>
                <span className="text-[10px] text-[var(--text-muted)]">1080P @ 25fps</span>
              </div>

              <div className="flex items-center gap-1.5 text-[var(--text-secondary)]">
                <ShieldAlert className="h-3.5 w-3.5" />
                <Maximize2 className="h-3.5 w-3.5 cursor-pointer hover:text-[var(--text-primary)]" />
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
