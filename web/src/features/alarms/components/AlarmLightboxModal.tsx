import React, { useCallback, useEffect, useRef, useState } from 'react'
import {
  Check,
  CheckCircle2,
  Download,
  ExternalLink,
  Loader2,
  Maximize,
  Minimize,
  RefreshCw,
  ShieldAlert,
  X,
} from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { evidenceApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import type { AlarmRecord } from '@/types'
import { useImageZoomPan } from '../hooks/useImageZoomPan'
import { ZoomControls } from './ZoomControls'
import {
  calculateFittedImageRect,
  formatFaceBBoxLabel,
  getBBoxStyle,
  type FittedImageRect,
  formatTimestamp,
  getRuleTypeLabel,
  parseTargetBBoxes,
} from '../utils'

export interface AlarmLightboxModalProps {
  alarm: AlarmRecord
  cameraName?: string
  onClose: () => void
  onToggleStatus: () => void
  onSelectCrop?: () => void
  t: (key: string, options?: Record<string, unknown>) => string
}

type FullImageStatus = 'unavailable' | 'loading' | 'loaded' | 'error'

export function AlarmLightboxModal({
  alarm,
  cameraName,
  onClose,
  onToggleStatus,
  onSelectCrop,
  t,
}: AlarmLightboxModalProps): React.ReactElement {
  const isProcessed = alarm.status === 'processed'
  const isCritical = alarm.severity === 'critical'
  const shouldReduce = useReducedMotion()

  const rootRef = useRef<HTMLDivElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const fullImageUrl = alarm.imageRelPath ? evidenceApi.getImageUrl(alarm.imageRelPath) : ''
  const previousFullImageUrl = useRef(fullImageUrl)
  const [fullImageStatus, setFullImageStatus] = useState<FullImageStatus>(
    fullImageUrl ? 'loading' : 'unavailable',
  )
  const [imageAttempt, setImageAttempt] = useState<number>(0)
  const [imgRect, setImgRect] = useState<FittedImageRect | null>(null)
  const isFullLoaded = fullImageStatus === 'loaded'
  const isFullLoading = fullImageStatus === 'loading'
  const isFullError = fullImageStatus === 'error'
  const fullImageSrc =
    imageAttempt > 0 && fullImageUrl
      ? `${fullImageUrl}${fullImageUrl.includes('?') ? '&' : '?'}retry=${imageAttempt}`
      : fullImageUrl

  // 浏览器原生全屏状态
  const [isFullscreen, setIsFullscreen] = useState<boolean>(false)

  const toggleBrowserFullscreen = useCallback(() => {
    if (!document.fullscreenElement) {
      rootRef.current?.requestFullscreen().catch(() => {})
    } else {
      document.exitFullscreen().catch(() => {})
    }
  }, [])

  // 缩放平移交互由统一 Hook 接管
  const { zoom, zoomIn, zoomOut, resetZoom, dragProps, containerCursorClass, transformStyle } =
    useImageZoomPan(containerRef, {
      onToggleStatus,
      onToggleFullscreen: toggleBrowserFullscreen,
    })

  const targetBBoxes = parseTargetBBoxes(alarm.bboxJson)
  const bodyBBox = targetBBoxes?.body ?? null
  const faceBBox = targetBBoxes?.face?.bbox ?? null

  useEffect(() => {
    if (previousFullImageUrl.current === fullImageUrl) return
    previousFullImageUrl.current = fullImageUrl
    setFullImageStatus(fullImageUrl ? 'loading' : 'unavailable')
    setImageAttempt(0)
    setImgRect(null)
    resetZoom()
  }, [fullImageUrl, resetZoom])

  // ESC 栈退出 (仅当未处于浏览器全屏时触发关闭，全屏时 ESC 会先退出全屏)
  useDismissStack(!isFullscreen, onClose)

  // 监听浏览器原生全屏状态
  useEffect(() => {
    const handleFullscreenChange = () => {
      setIsFullscreen(Boolean(document.fullscreenElement))
    }
    document.addEventListener('fullscreenchange', handleFullscreenChange)
    return () => document.removeEventListener('fullscreenchange', handleFullscreenChange)
  }, [])

  // 动态视口自适应尺寸计算
  const updateImageRect = useCallback(() => {
    const container = containerRef.current
    if (!container) return
    const img = container.querySelector('img.evidence-main-img') as HTMLImageElement | null
    if (!img || !img.naturalWidth || !img.naturalHeight) return
    setImgRect(
      calculateFittedImageRect(
        container.clientWidth,
        container.clientHeight,
        img.naturalWidth,
        img.naturalHeight,
      ),
    )
  }, [])

  useEffect(() => {
    const container = containerRef.current
    if (!container || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(updateImageRect)
    observer.observe(container)
    return () => observer.disconnect()
  }, [updateImageRect])

  const handleImageLoad = () => {
    setFullImageStatus('loaded')
    updateImageRect()
  }

  const handleImageError = () => {
    setFullImageStatus('error')
    setImgRect(null)
  }

  const handleRetry = () => {
    setImageAttempt((attempt) => attempt + 1)
    setFullImageStatus('loading')
  }

  const retryButton = (
    <button
      type="button"
      onClick={handleRetry}
      className="flex items-center gap-1.5 rounded-lg border border-white/20 px-2.5 py-1 text-[11px] text-white transition-colors hover:bg-white/10"
    >
      <RefreshCw className="h-3 w-3" />
      <span>{t('modal.retryImage')}</span>
    </button>
  )

  return (
    <motion.div
      ref={rootRef}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{
        duration: shouldReduce ? 0.1 : motionTokens.duration.fast,
        ease: motionTokens.easing.smooth,
      }}
      className="modal-backdrop modal-backdrop--immersive flex-col text-white select-none"
    >
      {/* 顶部悬浮磨砂指挥条 */}
      <div className="absolute inset-x-0 top-0 z-40 flex items-center justify-between border-b border-white/10 bg-gradient-to-b from-black/85 via-black/50 to-transparent px-6 py-3.5 backdrop-blur-md">
        <div className="flex items-center gap-3">
          <ShieldAlert
            className={`h-5 w-5 ${isCritical ? 'animate-pulse text-[var(--status-danger)]' : 'text-amber-500'}`}
          />
          <div className="flex flex-wrap items-center gap-2">
            <span
              className={`rounded-md px-2 py-0.5 text-[10px] font-bold tracking-wider uppercase shadow-xs backdrop-blur-md ${
                isCritical
                  ? 'animate-pulse bg-[var(--status-danger-solid)] text-white'
                  : 'bg-amber-500/90 text-white'
              }`}
            >
              {alarm.severity || 'WARNING'}
            </span>
            <h3 className="text-sm font-semibold tracking-wide text-white">
              {alarm.targetLabel} · {getRuleTypeLabel(alarm.ruleType, t)}
            </h3>
            <span className="hidden rounded-md border border-white/10 bg-black/40 px-2 py-0.5 font-mono text-xs text-zinc-400 sm:inline-block">
              {alarm.eventId}
            </span>
          </div>
        </div>

        {/* 右侧操作区：缩放控制、全屏、下载与关闭 */}
        <div className="flex items-center gap-2">
          {/* 浮动缩放控制条 */}
          {isFullLoaded && (
            <ZoomControls
              zoom={zoom}
              onZoomIn={zoomIn}
              onZoomOut={zoomOut}
              onResetZoom={resetZoom}
              t={t}
            />
          )}

          {alarm.imageRelPath && (
            <a
              href={evidenceApi.getImageUrl(alarm.imageRelPath)}
              download={`alarm_${alarm.eventId}.jpg`}
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-1.5 rounded-xl border border-white/15 bg-white/5 px-3 py-1.5 text-xs text-zinc-200 transition-all hover:bg-white/15 hover:text-white"
              title={t('viewImage')}
              aria-label={t('modal.fullImage')}
            >
              <Download className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">{t('modal.fullImage')}</span>
            </a>
          )}

          {/* 原生浏览器全屏切换 */}
          <button
            type="button"
            onClick={toggleBrowserFullscreen}
            className="flex items-center gap-1 rounded-xl border border-white/15 bg-white/5 px-2.5 py-1.5 text-xs text-zinc-200 transition-all hover:bg-white/15 hover:text-white"
            title={isFullscreen ? t('modal.exitFullscreen') : t('modal.enterFullscreen')}
            aria-label={isFullscreen ? t('modal.exitFullscreen') : t('modal.enterFullscreen')}
          >
            {isFullscreen ? (
              <Minimize className="h-3.5 w-3.5" />
            ) : (
              <Maximize className="h-3.5 w-3.5" />
            )}
          </button>

          {/* 关闭按钮 */}
          <button
            type="button"
            onClick={onClose}
            className="rounded-xl border border-white/15 bg-white/5 p-1.5 text-zinc-300 transition-all hover:bg-[var(--status-danger-soft)] hover:text-[var(--status-danger)] focus-visible:ring-2 focus-visible:ring-[var(--status-danger)]"
            title={`${t('modal.close')} (Esc)`}
            aria-label="Close"
          >
            <X className="h-5 w-5" />
          </button>
        </div>
      </div>

      {/* 中央全视口超清画布 (100% 视口满屏展开) */}
      <div
        ref={containerRef}
        {...dragProps}
        className={`relative flex h-full w-full flex-1 items-center justify-center overflow-hidden ${containerCursorClass}`}
      >
        {/* 加载占位与特写兜底 */}
        {fullImageStatus !== 'loaded' && alarm.cropImageRelPath && (
          <div className="absolute inset-0 flex flex-col items-center justify-center overflow-hidden bg-black/60 backdrop-blur-md">
            <img
              src={evidenceApi.getImageUrl(alarm.cropImageRelPath)}
              alt="Preview Placeholder"
              className="max-h-[60%] max-w-[60%] rounded-2xl object-contain opacity-75 shadow-2xl transition-opacity duration-150"
            />
            {isFullLoading && (
              <div className="absolute bottom-16 flex items-center gap-2 rounded-full border border-white/20 bg-black/70 px-4 py-1.5 text-xs text-zinc-200 shadow-lg backdrop-blur-md">
                <Loader2 className="h-4 w-4 animate-spin text-[var(--status-danger)]" />
                <span>{t('modal.loadingFullHd')}</span>
              </div>
            )}
            {isFullError && (
              <div className="absolute inset-x-4 bottom-16 flex flex-wrap items-center justify-center gap-2 rounded-xl border border-[var(--status-danger)]/30 bg-black/80 px-4 py-2 text-xs text-zinc-200 shadow-lg">
                <span>{t('modal.fullImageLoadFailed')}</span>
                {retryButton}
              </div>
            )}
          </div>
        )}

        {isFullLoading && !alarm.cropImageRelPath && (
          <div className="absolute inset-0 flex items-center justify-center">
            <Loader2 className="h-10 w-10 animate-spin text-[var(--status-danger)] opacity-70" />
          </div>
        )}

        {isFullError && !alarm.cropImageRelPath && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-black/80 p-4 text-center text-xs text-zinc-200">
            <span>{t('modal.fullImageLoadFailed')}</span>
            {retryButton}
          </div>
        )}

        {/* 图片与 BBox 联动缩放平移层 */}
        {alarm.imageRelPath ? (
          <div
            className="relative flex h-full w-full items-center justify-center will-change-transform"
            style={transformStyle}
          >
            <img
              src={fullImageSrc}
              alt="Alarm Fullscreen"
              fetchPriority="high"
              decoding="async"
              draggable={false}
              className={`evidence-main-img pointer-events-none max-h-full max-w-full object-contain transition-opacity duration-150 select-none ${
                isFullLoaded ? 'opacity-100' : 'opacity-0'
              }`}
              onLoad={handleImageLoad}
              onError={handleImageError}
            />

            {/* 目标人体 BBox 框 */}
            {bodyBBox && imgRect && isFullLoaded && (
              <div
                className="pointer-events-none absolute border-2 border-[var(--status-danger)] bg-[var(--status-danger)]/15 shadow-[0_0_15px_rgba(var(--status-danger-rgb),0.6)] transition-all"
                style={getBBoxStyle(bodyBBox, imgRect)}
              >
                <span className="absolute -top-5.5 left-0 rounded bg-[var(--status-danger-solid)] px-1.5 py-0.5 font-mono text-[9px] font-bold whitespace-nowrap text-white shadow-md">
                  #{alarm.trackId} {alarm.targetLabel} ({((alarm.confidence ?? 0) * 100).toFixed(0)}
                  %)
                </span>
              </div>
            )}

            {/* 目标人脸 BBox 框 */}
            {faceBBox && imgRect && isFullLoaded && (
              <div
                className="pointer-events-none absolute border-2 border-dashed border-purple-400 bg-purple-500/15 shadow-[0_0_12px_rgba(168,85,247,0.5)] transition-all"
                style={getBBoxStyle(faceBBox, imgRect)}
              >
                <span className="absolute -top-4.5 left-0 rounded bg-purple-600 px-1.5 py-0.5 font-mono text-[8px] font-bold whitespace-nowrap text-white shadow-md">
                  {formatFaceBBoxLabel(targetBBoxes?.face)}
                </span>
              </div>
            )}
          </div>
        ) : !alarm.cropImageRelPath ? (
          <div className="flex h-full w-full items-center justify-center font-mono text-sm text-zinc-500">
            {t('modal.noImage')}
          </div>
        ) : null}
      </div>

      {/* 底部悬浮智能信息胶囊坞 (Floating Bottom Dock) */}
      <motion.div
        initial={{ y: shouldReduce ? 0 : 20, opacity: 0 }}
        animate={{ y: 0, opacity: 1 }}
        exit={{ y: shouldReduce ? 0 : 20, opacity: 0 }}
        transition={{
          duration: shouldReduce ? 0.1 : motionTokens.duration.normal,
          ease: motionTokens.easing.smooth,
        }}
        className="absolute inset-x-0 bottom-5 z-40 mx-auto flex max-w-3xl flex-wrap items-center justify-between gap-3 rounded-2xl border border-white/15 bg-black/75 px-5 py-2.5 shadow-2xl backdrop-blur-xl"
      >
        <div className="flex items-center gap-4 text-xs">
          <div className="font-mono text-zinc-300">
            <span className="text-zinc-500">{t('modal.channel')}:</span>{' '}
            <span className="font-semibold text-white">{cameraName || alarm.cameraId}</span>
          </div>
          <div className="font-mono text-zinc-300">
            <span className="text-zinc-500">{t('modal.trackId')}:</span>{' '}
            <span>#{alarm.trackId}</span>
          </div>
          <div className="font-mono text-zinc-300">
            <span className="text-zinc-500">{t('modal.time')}:</span>{' '}
            <span>{formatTimestamp(alarm.occurredAt)}</span>
          </div>

          {alarm.cropImageRelPath && (
            <button
              type="button"
              onClick={onSelectCrop}
              className="flex items-center gap-1.5 rounded-xl border border-purple-500/30 bg-purple-500/15 px-2.5 py-1 font-mono text-xs font-medium text-purple-300 shadow-xs transition-all hover:bg-purple-500/25 hover:text-purple-200"
              title={t('modal.cropImage')}
            >
              <ExternalLink className="h-3 w-3" />
              <span>{t('card.siteCrop')}</span>
            </button>
          )}
        </div>

        {/* 状态流转操作 */}
        <div className="flex items-center gap-2">
          <motion.button
            type="button"
            whileTap={{ scale: 0.96 }}
            onClick={onToggleStatus}
            className={`flex items-center gap-1.5 rounded-xl px-3.5 py-1.5 text-xs font-semibold shadow-md transition-all ${
              isProcessed
                ? 'border border-emerald-500/30 bg-emerald-500/20 text-emerald-300 hover:bg-emerald-500/30'
                : 'border border-[var(--status-danger)]/30 bg-[var(--status-danger)]/20 text-[var(--status-danger)] hover:bg-[var(--status-danger-soft)]'
            }`}
          >
            {isProcessed ? (
              <>
                <CheckCircle2 className="h-3.5 w-3.5" />
                <span>{t('card.processed')}</span>
              </>
            ) : (
              <>
                <Check className="h-3.5 w-3.5" />
                <span>{t('card.markProcessed')}</span>
              </>
            )}
          </motion.button>
        </div>
      </motion.div>

      {/* 底部轻量提示 */}
      <div className="pointer-events-none absolute bottom-1.5 left-1/2 z-40 -translate-x-1/2 font-mono text-[10px] text-zinc-500">
        {t('modal.escHint')} · {t('modal.zoomHint')} · {t('modal.toggleStatusShort')} ·{' '}
        {t('modal.fullscreenShort')}
      </div>
    </motion.div>
  )
}
