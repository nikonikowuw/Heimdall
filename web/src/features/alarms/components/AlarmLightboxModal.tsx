import React, { useCallback, useEffect, useRef, useState } from 'react'
import {
  Check,
  CheckCircle2,
  Download,
  ExternalLink,
  Film,
  ImageIcon,
  Loader2,
  Maximize,
  Minimize,
  RefreshCw,
  ShieldAlert,
  X,
} from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { useFocusTrap } from '@/hooks/use-focus-trap'
import { evidenceApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import type { AlarmRecord } from '@/types'
import { useEventRecording } from '../hooks/useEventRecording'
import { useImageZoomPan } from '../hooks/useImageZoomPan'
import { exportRecordingAsMp4 } from '../recordingExport'
import { EventRecordingPlayer } from './EventRecordingPlayer'
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
  const { t: tr } = useTranslation('recording')
  const isProcessed = alarm.status === 'processed'
  const isCritical = alarm.severity === 'critical'
  const shouldReduce = useReducedMotion()

  // 事件关联录像：仅当后端已保存过该事件的片段时，才提供回放入口
  const {
    recording,
    error: recordingError,
    retry: retryRecording,
  } = useEventRecording('alarm', alarm.eventId)
  const [mediaMode, setMediaMode] = useState<'image' | 'video'>('image')
  const [exportState, setExportState] = useState<'idle' | 'exporting' | 'failed'>('idle')
  const activeEventOffsetMs =
    recording?.events.find((item) => item.eventId === alarm.eventId)?.offsetMs ?? 0

  // 换告警时回到照片模式，避免沿用上一条事件的媒体选择
  const [mediaAlarmId, setMediaAlarmId] = useState(alarm.eventId)
  if (mediaAlarmId !== alarm.eventId) {
    setMediaAlarmId(alarm.eventId)
    setMediaMode('image')
    setExportState('idle')
  }

  const handleExport = useCallback(() => {
    if (!recording || exportState === 'exporting') return
    setExportState('exporting')
    exportRecordingAsMp4(recording)
      .then(() => setExportState('idle'))
      .catch(() => setExportState('failed'))
  }, [recording, exportState])

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
      // 视频模式下把滚轮与快捷键交还给播放器
      enabled: mediaMode === 'image',
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

  // 沉浸式查看器同样需要焦点约束：把 Tab 限制在浮层内并在关闭后归还焦点
  useFocusTrap(true, rootRef)

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
    // setImgRect 来自 useState，引用稳定；显式列入依赖是 React Compiler 的要求 ——
    // 它依推断出的依赖必须出现在源码依赖数组中，否则整个组件会被跳过优化。
  }, [setImgRect])

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
      role="dialog"
      aria-modal="true"
      aria-label={`${t('modal.fullImage')}: ${alarm.targetLabel}`}
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
            className={`h-5 w-5 ${isCritical ? 'animate-pulse text-[var(--status-danger)]' : 'text-status-warning'}`}
          />
          <div className="flex flex-wrap items-center gap-2">
            <span
              className={`rounded-md px-2 py-0.5 text-[10px] font-bold tracking-wider uppercase shadow-xs backdrop-blur-md ${
                isCritical
                  ? 'animate-pulse bg-[var(--status-danger-solid)] text-white'
                  : 'bg-[var(--status-warning-solid)]/90 text-white'
              }`}
            >
              {alarm.severity || 'WARNING'}
            </span>
            <h3 className="text-sm font-semibold tracking-wide text-white">
              {alarm.targetLabel} · {getRuleTypeLabel(alarm.ruleType, t)}
            </h3>
            <span className="hidden rounded-md border border-white/10 bg-black/40 px-2 py-0.5 font-mono text-xs text-[var(--text-secondary)] sm:inline-block">
              {alarm.eventId}
            </span>
          </div>
        </div>

        {/* 右侧操作区：媒体切换、缩放控制、全屏、下载与关闭 */}
        <div className="flex items-center gap-2">
          {/* 录像查询失败：给出可操作的重试，不静默当作无录像 */}
          {recordingError && !recording && (
            <button
              type="button"
              onClick={retryRecording}
              title={recordingError}
              className="flex items-center gap-1.5 rounded-xl border border-[var(--status-danger)]/40 bg-[var(--status-danger)]/20 px-3 py-1.5 text-xs text-[var(--status-danger)] transition-all hover:bg-[var(--status-danger)]/30"
            >
              <RefreshCw className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">
                {tr('playback.loadRetry', { defaultValue: '录像加载失败，重试' })}
              </span>
            </button>
          )}
          {/* 媒体切换：仅当该事件已保存录像时出现，默认仍看证据图 */}
          {recording && (
            <div className="flex items-center gap-0.5 rounded-xl border border-white/15 bg-white/5 p-0.5 text-xs">
              <button
                type="button"
                onClick={() => setMediaMode('image')}
                aria-pressed={mediaMode === 'image'}
                className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 transition-colors ${
                  mediaMode === 'image'
                    ? 'bg-white/15 text-white'
                    : 'text-[var(--text-secondary)] hover:text-white'
                }`}
              >
                <ImageIcon className="h-3.5 w-3.5" />
                <span className="hidden sm:inline">
                  {tr('playback.mediaImage', { defaultValue: '证据图' })}
                </span>
              </button>
              <button
                type="button"
                onClick={() => setMediaMode('video')}
                aria-pressed={mediaMode === 'video'}
                className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 transition-colors ${
                  mediaMode === 'video'
                    ? 'bg-white/15 text-white'
                    : 'text-[var(--text-secondary)] hover:text-white'
                }`}
              >
                <Film className="h-3.5 w-3.5" />
                <span className="hidden sm:inline">
                  {tr('playback.mediaVideo', { defaultValue: '录像回放' })}
                </span>
              </button>
            </div>
          )}

          {/* 浮动缩放控制条（仅图片模式） */}
          {mediaMode === 'image' && isFullLoaded && (
            <ZoomControls
              zoom={zoom}
              onZoomIn={zoomIn}
              onZoomOut={zoomOut}
              onResetZoom={resetZoom}
              t={t}
            />
          )}

          {mediaMode === 'video' && recording ? (
            <button
              type="button"
              disabled={exportState === 'exporting'}
              onClick={handleExport}
              title={exportState === 'failed' ? tr('playback.exportFailed') : tr('playback.export')}
              className={`flex items-center gap-1.5 rounded-xl border px-3 py-1.5 text-xs transition-all disabled:opacity-60 ${
                exportState === 'failed'
                  ? 'border-[var(--status-danger)]/40 bg-[var(--status-danger)]/20 text-[var(--status-danger)]'
                  : 'border-white/15 bg-white/5 text-[var(--text-primary)] hover:bg-white/15 hover:text-white'
              }`}
            >
              {exportState === 'exporting' ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Download className="h-3.5 w-3.5" />
              )}
              <span className="hidden sm:inline">
                {exportState === 'exporting'
                  ? tr('playback.exporting')
                  : exportState === 'failed'
                    ? tr('playback.exportRetry')
                    : tr('playback.export')}
              </span>
            </button>
          ) : alarm.imageRelPath ? (
            <a
              href={evidenceApi.getImageUrl(alarm.imageRelPath)}
              download={`alarm_${alarm.eventId}.jpg`}
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-1.5 rounded-xl border border-white/15 bg-white/5 px-3 py-1.5 text-xs text-[var(--text-primary)] transition-all hover:bg-white/15 hover:text-white"
              title={t('viewImage')}
              aria-label={t('modal.fullImage')}
            >
              <Download className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">{t('modal.fullImage')}</span>
            </a>
          ) : null}

          {/* 原生浏览器全屏切换 */}
          <button
            type="button"
            onClick={toggleBrowserFullscreen}
            className="flex items-center gap-1 rounded-xl border border-white/15 bg-white/5 px-2.5 py-1.5 text-xs text-[var(--text-primary)] transition-all hover:bg-white/15 hover:text-white"
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
            className="rounded-xl border border-white/15 bg-white/5 p-1.5 text-[var(--text-primary)] transition-all hover:bg-[var(--status-danger-soft)] hover:text-[var(--status-danger)] focus-visible:ring-2 focus-visible:ring-[var(--status-danger)]"
            title={`${t('modal.close')} (Esc)`}
            aria-label={t('modal.close', { defaultValue: '关闭' })}
          >
            <X className="h-5 w-5" />
          </button>
        </div>
      </div>

      {/* 中央全视口超清画布 (100% 视口满屏展开) */}
      <div
        ref={containerRef}
        {...(mediaMode === 'image' ? dragProps : {})}
        className={`relative flex h-full w-full flex-1 items-center justify-center overflow-hidden ${
          mediaMode === 'image' ? containerCursorClass : ''
        }`}
      >
        {mediaMode === 'video' && recording ? (
          <div className="flex h-full w-full items-center justify-center px-4 pt-16 pb-24 sm:px-6">
            <EventRecordingPlayer recording={recording} initialOffsetMs={activeEventOffsetMs} />
          </div>
        ) : (
          <>
            {/* 加载占位与特写兜底 */}
            {fullImageStatus !== 'loaded' && alarm.cropImageRelPath && (
              <div className="absolute inset-0 flex flex-col items-center justify-center overflow-hidden bg-black/60 backdrop-blur-md">
                <img
                  src={evidenceApi.getImageUrl(alarm.cropImageRelPath)}
                  alt="Preview Placeholder"
                  className="max-h-[60%] max-w-[60%] rounded-2xl object-contain opacity-75 shadow-2xl transition-opacity duration-150"
                />
                {isFullLoading && (
                  <div className="absolute bottom-16 flex items-center gap-2 rounded-full border border-white/20 bg-black/70 px-4 py-1.5 text-xs text-[var(--text-primary)] shadow-lg backdrop-blur-md">
                    <Loader2 className="h-4 w-4 animate-spin text-[var(--status-danger)]" />
                    <span>{t('modal.loadingFullHd')}</span>
                  </div>
                )}
                {isFullError && (
                  <div className="absolute inset-x-4 bottom-16 flex flex-wrap items-center justify-center gap-2 rounded-xl border border-[var(--status-danger)]/30 bg-black/80 px-4 py-2 text-xs text-[var(--text-primary)] shadow-lg">
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
              <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-black/80 p-4 text-center text-xs text-[var(--text-primary)]">
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
                      #{alarm.trackId} {alarm.targetLabel} (
                      {((alarm.confidence ?? 0) * 100).toFixed(0)}
                      %)
                    </span>
                  </div>
                )}

                {/* 目标人脸 BBox 框 */}
                {faceBBox && imgRect && isFullLoaded && (
                  <div
                    className="border-marker-border bg-marker-soft pointer-events-none absolute border-2 border-dashed shadow-[0_0_12px_rgba(var(--marker-rgb),0.5)] transition-all"
                    style={getBBoxStyle(faceBBox, imgRect)}
                  >
                    <span className="bg-marker-solid absolute -top-4.5 left-0 rounded px-1.5 py-0.5 font-mono text-[8px] font-bold whitespace-nowrap text-white shadow-md">
                      {formatFaceBBoxLabel(targetBBoxes?.face)}
                    </span>
                  </div>
                )}
              </div>
            ) : !alarm.cropImageRelPath ? (
              <div className="flex h-full w-full items-center justify-center font-mono text-sm text-[var(--text-muted)]">
                {t('modal.noImage')}
              </div>
            ) : null}
          </>
        )}
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
          <div className="font-mono text-[var(--text-primary)]">
            <span className="text-[var(--text-muted)]">{t('modal.channel')}:</span>{' '}
            <span className="font-semibold text-white">{cameraName || alarm.cameraId}</span>
          </div>
          <div className="font-mono text-[var(--text-primary)]">
            <span className="text-[var(--text-muted)]">{t('modal.trackId')}:</span>{' '}
            <span>#{alarm.trackId}</span>
          </div>
          <div className="font-mono text-[var(--text-primary)]">
            <span className="text-[var(--text-muted)]">{t('modal.time')}:</span>{' '}
            <span>{formatTimestamp(alarm.occurredAt)}</span>
          </div>

          {alarm.cropImageRelPath && (
            <button
              type="button"
              onClick={onSelectCrop}
              className="border-marker/30 bg-marker-soft text-marker hover:bg-marker/25 hover:text-marker flex items-center gap-1.5 rounded-xl border px-2.5 py-1 font-mono text-xs font-medium shadow-xs transition-all"
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
                ? 'border-status-success/30 bg-status-success/20 text-status-success hover:bg-status-success/30 border'
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
      <div className="pointer-events-none absolute bottom-1.5 left-1/2 z-40 -translate-x-1/2 font-mono text-[10px] text-[var(--text-muted)]">
        {t('modal.escHint')}
        {mediaMode === 'image' && <> · {t('modal.zoomHint')}</>} · {t('modal.toggleStatusShort')} ·{' '}
        {t('modal.fullscreenShort')}
      </div>
    </motion.div>
  )
}
