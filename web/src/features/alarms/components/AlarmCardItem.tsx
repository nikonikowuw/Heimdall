import React, { useCallback, useEffect, useRef, useState } from 'react'
import { Check, CheckCircle2, Clock, ExternalLink } from 'lucide-react'
import { evidenceApi } from '@/lib/api'
import type { AlarmRecord } from '@/types'
import { formatTimestamp, getRuleTypeLabel, preloadImage } from '../utils'

export interface AlarmCardItemProps {
  alarm: AlarmRecord
  cameraName?: string
  isSelected?: boolean
  onToggleSelect?: (id: number, selected: boolean) => void
  onSelect: (alarm: AlarmRecord) => void
  onSelectCrop: (alarm: AlarmRecord) => void
  onToggleStatus: (alarm: AlarmRecord) => void
  t: (key: string, options?: Record<string, unknown>) => string
}

export const AlarmCardItem = React.memo(function AlarmCardItem({
  alarm,
  cameraName,
  isSelected = false,
  onToggleSelect,
  onSelect,
  onSelectCrop,
  onToggleStatus,
  t,
}: AlarmCardItemProps): React.ReactElement {
  const isProcessed = alarm.status === 'processed'
  const isCritical = alarm.severity === 'critical'
  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [imageLoaded, setImageLoaded] = useState(false)
  const [imageError, setImageError] = useState(false)

  // 悬停防抖：停顿 200ms 以上才判定为有查看意图，触发全景大图预加载，避免滑动时的网络请求风暴
  const handlePointerEnter = useCallback(() => {
    if (!alarm.imageRelPath) return
    hoverTimerRef.current = setTimeout(() => {
      preloadImage(evidenceApi.getImageUrl(alarm.imageRelPath))
    }, 200)
  }, [alarm.imageRelPath])

  const handlePointerLeave = useCallback(() => {
    if (hoverTimerRef.current) {
      clearTimeout(hoverTimerRef.current)
      hoverTimerRef.current = null
    }
  }, [])

  // 组件卸载时确保定时器销毁，防范异步微任务泄露
  useEffect(() => {
    return () => {
      if (hoverTimerRef.current) {
        clearTimeout(hoverTimerRef.current)
        hoverTimerRef.current = null
      }
    }
  }, [])

  // 键盘操作支持：Enter 打开大图，Space 切换核验状态（严格限制仅卡片自身聚焦响应，防止子控件冒泡冲突）
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.target !== e.currentTarget) return

    if (e.key === 'Enter') {
      e.preventDefault()
      onSelect(alarm)
    } else if (e.key === ' ') {
      e.preventDefault()
      onToggleStatus(alarm)
    }
  }

  const thumbUrl = alarm.cropImageRelPath || alarm.imageRelPath

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onSelect(alarm)}
      onKeyDown={handleKeyDown}
      onPointerEnter={handlePointerEnter}
      onPointerLeave={handlePointerLeave}
      className={`group relative flex cursor-pointer flex-col overflow-hidden rounded-2xl border bg-[var(--bg-surface)] shadow-xs transition-all duration-200 hover:shadow-md focus-visible:ring-2 focus-visible:ring-[var(--accent)] focus-visible:outline-none ${
        isSelected
          ? 'border-[var(--accent)] ring-1 ring-[var(--accent)]/50'
          : isProcessed
            ? 'border-[var(--border)] opacity-75 hover:opacity-100'
            : isCritical
              ? 'border-[var(--status-danger)]/70 shadow-[0_0_12px_var(--status-danger-soft)] hover:border-[var(--status-danger)]'
              : 'border-[var(--status-warning-border)] hover:border-[var(--status-warning)]'
      }`}
    >
      <div className="relative aspect-video w-full overflow-hidden bg-[var(--video-surface)]">
        {/* 多选勾选复选框 */}
        {onToggleSelect && (
          <div
            className="absolute top-2 right-2 z-20"
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => e.stopPropagation()}
          >
            <input
              type="checkbox"
              checked={isSelected}
              onChange={(e) => onToggleSelect(alarm.id, e.target.checked)}
              className="h-4 w-4 cursor-pointer rounded border-[var(--border)] bg-black/60 text-[var(--accent)] transition-transform hover:scale-110 focus:ring-1 focus:ring-[var(--accent)]"
              aria-label={t('card.selectAlarm', { id: alarm.eventId })}
            />
          </div>
        )}

        {/* 缩略图渐进式加载与骨架占位 */}
        {thumbUrl && !imageError ? (
          <>
            {!imageLoaded && (
              <div className="absolute inset-0 flex animate-pulse items-center justify-center bg-[var(--overlay-scrim)] font-mono text-[10px] text-[var(--text-muted)]">
                LOADING...
              </div>
            )}
            <img
              src={evidenceApi.getImageUrl(thumbUrl)}
              alt={alarm.eventId}
              loading="lazy"
              decoding="async"
              onLoad={() => setImageLoaded(true)}
              onError={() => setImageError(true)}
              className={`h-full w-full object-cover transition-all duration-300 group-hover:scale-105 ${
                imageLoaded ? 'opacity-100' : 'opacity-0'
              }`}
            />
          </>
        ) : (
          <div className="flex h-full w-full items-center justify-center font-mono text-xs text-[var(--text-muted)]">
            {t('card.noImage')}
          </div>
        )}

        {/* 现场特写快速入口 */}
        {alarm.cropImageRelPath && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onSelectCrop(alarm)
            }}
            className="absolute right-2 bottom-2 z-20 flex items-center gap-1 rounded-lg border border-white/40 bg-black/80 px-2 py-1 text-[10px] font-medium text-white shadow-md backdrop-blur-xs transition-all duration-200 hover:border-white/70 hover:shadow-lg hover:shadow-black/40 focus-visible:ring-1 focus-visible:ring-white"
            title={t('modal.cropImage')}
            aria-label={t('modal.cropImage')}
          >
            <ExternalLink className="h-3 w-3" />
            <span>{t('card.siteCrop')}</span>
          </button>
        )}

        {/* 左上角严重级别与规则徽章 */}
        <div className="absolute top-2 left-2 flex items-center gap-1.5">
          <span
            className={`rounded-md px-2 py-0.5 text-[10px] font-bold tracking-wider uppercase shadow-xs backdrop-blur-md ${
              isCritical
                ? 'animate-pulse bg-[var(--status-danger)] text-white'
                : 'bg-[var(--status-warning)] text-white'
            }`}
          >
            {alarm.severity || 'WARNING'}
          </span>
          <span className="rounded-md bg-black/60 px-1.5 py-0.5 font-mono text-[10px] text-white backdrop-blur-xs">
            {getRuleTypeLabel(alarm.ruleType, t)}
          </span>
        </div>
      </div>

      <div className="space-y-2 p-3.5 text-xs">
        <div className="flex items-center justify-between">
          <span className="font-semibold text-[var(--text-primary)]">
            {t('card.target')}: {alarm.targetLabel}
          </span>
          <span className="font-mono text-[11px] font-semibold text-[var(--accent)]">
            {((alarm.confidence ?? 0) * 100).toFixed(0)}% {t('card.confidence')}
          </span>
        </div>

        <div className="flex items-center justify-between font-mono text-[11px] text-[var(--text-muted)]">
          <span
            className="max-w-[140px] truncate font-sans font-medium text-[var(--text-secondary)]"
            title={cameraName || alarm.cameraId}
          >
            {cameraName || alarm.cameraId}
          </span>
          <span className="flex items-center gap-1">
            <Clock className="h-3 w-3 opacity-60" />
            <span>{formatTimestamp(alarm.occurredAt)}</span>
          </span>
        </div>

        {/* 底部操作与核验状态流转按钮 */}
        <div className="flex items-center justify-between gap-2 border-t border-[var(--border)]/60 pt-2.5">
          <div className="min-w-0 flex-1 font-mono text-[10px] break-all text-[var(--text-muted)]">
            ID: {alarm.eventId}
          </div>

          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onToggleStatus(alarm)
            }}
            className={`flex shrink-0 items-center gap-1 rounded-lg px-2.5 py-1 text-[11px] font-semibold transition-all duration-150 focus-visible:ring-1 focus-visible:ring-[var(--accent)] ${
              isProcessed
                ? 'border border-[var(--status-success-border)] bg-[var(--status-success-soft)] text-[var(--status-success)] hover:bg-[var(--status-success-soft)]'
                : 'border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] text-[var(--status-danger)] hover:bg-[var(--status-danger-soft)]'
            }`}
            aria-label={isProcessed ? t('card.processed') : t('card.markProcessed')}
          >
            {isProcessed ? (
              <>
                <CheckCircle2 className="h-3 w-3" />
                <span>{t('card.processed')}</span>
              </>
            ) : (
              <>
                <Check className="h-3 w-3" />
                <span>{t('card.markProcessed')}</span>
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  )
})
