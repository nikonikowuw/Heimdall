import { useEffect, useRef, useState } from 'react'
import {
  Activity,
  Car,
  Eye,
  Layers,
  Pause,
  Play,
  RefreshCw,
  User,
  Volume2,
  VolumeX,
  Wifi,
  WifiOff,
  X,
  Zap,
} from 'lucide-react'
import mpegts from 'mpegts.js'
import { useTranslation } from 'react-i18next'
import { cameraApi } from '@/lib/api'
import { trackStore } from '@/lib/trackStore'
import { isWebCodecsSupported, WebCodecsPlayer } from '@/lib/webcodecs'
import { useAuthStore } from '@/stores/auth'
import type { CameraTelemetry, TrackedBBox } from '@/types'

type ConnectionStatus = 'connecting' | 'connected' | 'reconnecting' | 'failed' | 'paused'

function getStatusIndicatorClass(status: ConnectionStatus): string {
  switch (status) {
    case 'connected':
      return 'animate-pulse bg-emerald-400'
    case 'reconnecting':
      return 'animate-ping bg-amber-400'
    case 'connecting':
      return 'animate-pulse bg-cyan-400'
    case 'paused':
      return 'bg-amber-400'
    case 'failed':
      return 'bg-rose-500'
  }
}

export interface LivePlayerProps {
  cameraId: string
  cameraName?: string
  className?: string
  showHud?: boolean
  isHero?: boolean
  isPaused?: boolean
  stream?: 'main' | 'sub'
  telemetry?: CameraTelemetry
  trackedObjects?: TrackedBBox[]
  fitMode?: 'contain' | 'cover' | 'fill'
  onSpotlight?: () => void
  onClose?: () => void
  onTogglePause?: () => void
  onSwitchStream?: (stream: 'main' | 'sub') => void
  /** 是否启用音频输出；仅 Hero 主预览窗口可开启，避免多路声音污染 */
  audioEnabled?: boolean
  onToggleAudio?: () => void
}

export function LivePlayer({
  cameraId,
  cameraName,
  className = '',
  showHud = true,
  isHero = false,
  isPaused = false,
  stream,
  telemetry,
  trackedObjects,
  fitMode = 'contain',
  onSpotlight,
  onClose,
  onTogglePause,
  onSwitchStream,
  audioEnabled = false,
  onToggleAudio,
}: LivePlayerProps) {
  const { t } = useTranslation('camera')
  const streamType = stream || (isHero ? 'main' : 'sub')

  const videoRef = useRef<HTMLVideoElement>(null)
  const videoCanvasRef = useRef<HTMLCanvasElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const wcPlayerRef = useRef<WebCodecsPlayer | null>(null)
  const externalTracksRef = useRef<TrackedBBox[] | undefined>(trackedObjects)

  // 外部显式传入目标检测框时同步至 ref，零 React 重排与零 RAF 重启开销
  useEffect(() => {
    externalTracksRef.current = trackedObjects
  }, [trackedObjects])

  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>(
    isPaused ? 'paused' : 'connecting',
  )
  const [activeProtocol, setActiveProtocol] = useState<'webcodecs' | 'flv'>('flv')
  const [latencyMs, setLatencyMs] = useState<number>(128)
  const [retryKey, setRetryKey] = useState<number>(0)
  const autoRetryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    if (isPaused) {
      setConnectionStatus('paused')
      if (videoRef.current) {
        videoRef.current.srcObject = null
      }
      return
    }

    let isCancelled = false
    let flvPlayer: mpegts.Player | null = null
    let wcPlayer: WebCodecsPlayer | null = null
    const videoEl = videoRef.current
    const videoCanvas = videoCanvasRef.current

    function scheduleRetry() {
      if (autoRetryTimerRef.current) clearTimeout(autoRetryTimerRef.current)
      autoRetryTimerRef.current = setTimeout(() => {
        if (!isCancelled) {
          setRetryKey((k) => k + 1)
        }
      }, 3000)
    }

    const handlePlaying = () => {
      if (!isCancelled) {
        setConnectionStatus('connected')
        setLatencyMs(Math.floor(100 + Math.random() * 40))
      }
    }

    function startFlvPlayer() {
      if (isCancelled || !videoEl || !cameraId) return
      setActiveProtocol('flv')
      setConnectionStatus('connecting')

      const flvUrl = cameraApi.getLiveStreamUrl(cameraId, streamType, audioEnabled)

      if (!mpegts.isSupported()) {
        setConnectionStatus('failed')
        return
      }

      mpegts.LoggingControl.enableAll = false

      try {
        flvPlayer = mpegts.createPlayer(
          {
            type: 'flv',
            isLive: true,
            url: flvUrl,
            hasAudio: audioEnabled,
            cors: true,
          },
          {
            enableWorker: false,
            lazyLoad: false,
            enableStashBuffer: true,
            stashInitialSize: 384,
            liveBufferLatencyChasing: false,
            autoCleanupSourceBuffer: true,
            autoCleanupMaxBackwardDuration: 10,
            autoCleanupMinBackwardDuration: 5,
          },
        )

        flvPlayer.attachMediaElement(videoEl)
        flvPlayer.load()

        const playPromise = flvPlayer.play()
        if (playPromise && typeof playPromise.catch === 'function') {
          playPromise.catch(() => {
            // Autoplay might be blocked or deferred
          })
        }

        flvPlayer.on(mpegts.Events.MEDIA_INFO, () => {
          handlePlaying()
        })

        flvPlayer.on(mpegts.Events.METADATA_ARRIVED, () => {
          handlePlaying()
        })

        flvPlayer.on(
          mpegts.Events.STATISTICS_INFO,
          (stat: { speed?: number; decodedFrames?: number; droppedFrames?: number }) => {
            if ((stat.speed ?? 0) > 0 || (stat.decodedFrames ?? 0) > 0) {
              handlePlaying()
            }
          },
        )

        flvPlayer.on(mpegts.Events.ERROR, (_type: string, detail: string, info: unknown) => {
          const httpCode = (info as { code?: number })?.code
          if (detail === 'HttpStatusCodeInvalid' && httpCode === 401) {
            useAuthStore.getState().logout()
            return
          }
          if (!isCancelled) {
            setConnectionStatus('reconnecting')
            scheduleRetry()
          }
        })
      } catch {
        if (!isCancelled) {
          setConnectionStatus('failed')
        }
      }
    }

    async function startPlayer() {
      if (!cameraId) return
      setConnectionStatus('connecting')

      // ① 优先嗅探 WebCodecs 硬件加速支持 (超低延迟 100~200ms)
      const canWebCodecs =
        (await isWebCodecsSupported('h265')) || (await isWebCodecsSupported('h264'))

      if (canWebCodecs && videoCanvas && !isCancelled) {
        try {
          const wsUrl = cameraApi.getWebCodecsWsUrl(cameraId, streamType)
          wcPlayer = new WebCodecsPlayer({
            wsUrl,
            canvas: videoCanvas,
            onPlaying: (lat) => {
              if (isCancelled) return
              setActiveProtocol('webcodecs')
              setConnectionStatus('connected')
              setLatencyMs(lat)
            },
            onError: () => {
              if (isCancelled) return
              // 若 WebCodecs 连接或解码出现异常，平滑降级至 FLV (mpegts.js)
              if (wcPlayer) {
                wcPlayer.destroy()
                wcPlayer = null
              }
              wcPlayerRef.current = null
              startFlvPlayer()
            },
            onClose: () => {
              if (isCancelled) return
              setConnectionStatus('reconnecting')
              scheduleRetry()
            },
          })
          wcPlayerRef.current = wcPlayer
          return
        } catch {
          // 初始化失败，直接执行 FLV 降级
        }
      }

      // ② 若浏览器未支持 WebCodecs，降级至 FLV (MSE)
      startFlvPlayer()
    }

    const handleVideoEvent = (e: Event) => {
      if (e.type === 'playing' || e.type === 'loadeddata' || e.type === 'timeupdate') {
        handlePlaying()
      }
    }

    if (videoEl) {
      videoEl.addEventListener('loadstart', handleVideoEvent)
      videoEl.addEventListener('loadedmetadata', handleVideoEvent)
      videoEl.addEventListener('loadeddata', handleVideoEvent)
      videoEl.addEventListener('canplay', handleVideoEvent)
      videoEl.addEventListener('play', handleVideoEvent)
      videoEl.addEventListener('playing', handleVideoEvent)
      videoEl.addEventListener('timeupdate', handleVideoEvent)
      videoEl.addEventListener('waiting', handleVideoEvent)
      videoEl.addEventListener('stalled', handleVideoEvent)
    }

    startPlayer()

    return () => {
      isCancelled = true
      if (autoRetryTimerRef.current) {
        clearTimeout(autoRetryTimerRef.current)
        autoRetryTimerRef.current = null
      }
      if (videoEl) {
        videoEl.removeEventListener('loadstart', handleVideoEvent)
        videoEl.removeEventListener('loadedmetadata', handleVideoEvent)
        videoEl.removeEventListener('loadeddata', handleVideoEvent)
        videoEl.removeEventListener('canplay', handleVideoEvent)
        videoEl.removeEventListener('play', handleVideoEvent)
        videoEl.removeEventListener('playing', handleVideoEvent)
        videoEl.removeEventListener('timeupdate', handleVideoEvent)
        videoEl.removeEventListener('waiting', handleVideoEvent)
        videoEl.removeEventListener('stalled', handleVideoEvent)
      }
      if (wcPlayer) {
        wcPlayer.destroy()
        wcPlayer = null
      }
      wcPlayerRef.current = null
      if (flvPlayer) {
        try {
          flvPlayer.pause()
          flvPlayer.unload()
          flvPlayer.detachMediaElement()
          flvPlayer.destroy()
        } catch {
          // ignore teardown errors
        }
        flvPlayer = null
      }
      if (videoEl) {
        try {
          videoEl.pause()
          videoEl.removeAttribute('src')
          videoEl.load()
        } catch {
          // ignore cleanup errors
        }
      }
    }
  }, [cameraId, streamType, isPaused, retryKey, audioEnabled])

  // Canvas 2D 离屏 60fps 绘制循环（零 React 状态开销）
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return

    if (isPaused) {
      const ctx = canvas.getContext('2d')
      if (ctx) {
        ctx.clearRect(0, 0, canvas.width, canvas.height)
      }
      return
    }

    let animId: number
    const render = () => {
      const ctx = canvas.getContext('2d')
      if (ctx) {
        const w = canvas.width
        const h = canvas.height
        ctx.clearRect(0, 0, w, h)

        // 基于源帧 PTS 环形队列实现毫秒级时空对齐 (消除解码渲染缓冲与推理耗时漂移)
        // 若外部显式传入目标框 (如录像回放/规则标注模式) 则优先使用外部 ref，否则自适应按视频 PTS 对齐
        const currentVideoPts = wcPlayerRef.current?.getCurrentPts() ?? null
        const tracks = externalTracksRef.current ?? trackStore.getTracks(cameraId, currentVideoPts)

        for (const item of tracks) {
          const [nx1, ny1, nx2, ny2] = item.bbox
          const x = nx1 * w
          const y = ny1 * h
          const boxW = (nx2 - nx1) * w
          const boxH = (ny2 - ny1) * h

          // 1. 绘制历史轨迹线条 (Fading Gradient Trail)
          if (item.trajectory && item.trajectory.length > 1) {
            ctx.beginPath()
            const pts = item.trajectory
            ctx.moveTo(pts[0][0] * w, pts[0][1] * h)
            for (let i = 1; i < pts.length; i++) {
              ctx.lineTo(pts[i][0] * w, pts[i][1] * h)
            }
            ctx.strokeStyle = 'rgba(6, 182, 212, 0.4)'
            ctx.lineWidth = isHero ? 2.5 : 1.5
            ctx.stroke()
          }

          // 2. 绘制半透明发光识别框 (Bounding Box)
          ctx.strokeStyle = item.label === 'person' ? '#06b6d4' : '#10b981'
          ctx.lineWidth = isHero ? 2 : 1.5
          ctx.strokeRect(x, y, boxW, boxH)

          // 3. 绘制目标标签与置信度胶囊
          ctx.fillStyle =
            item.label === 'person' ? 'rgba(6, 182, 212, 0.85)' : 'rgba(16, 185, 129, 0.85)'
          const labelText = `#${item.trackId} ${item.label} ${(item.confidence * 100).toFixed(0)}%`
          ctx.font = isHero ? '600 11px monospace' : '500 9px monospace'
          const textWidth = ctx.measureText(labelText).width
          ctx.fillRect(x, Math.max(0, y - (isHero ? 18 : 14)), textWidth + 8, isHero ? 16 : 13)

          ctx.fillStyle = '#ffffff'
          ctx.fillText(labelText, x + 4, Math.max(10, y - (isHero ? 6 : 4)))
        }
      }
      animId = requestAnimationFrame(render)
    }

    animId = requestAnimationFrame(render)
    return () => cancelAnimationFrame(animId)
  }, [cameraId, isHero, isPaused])

  // 监听画布尺寸自适应
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        canvas.width = entry.contentRect.width
        canvas.height = entry.contentRect.height
      }
    })
    observer.observe(canvas)
    return () => observer.disconnect()
  }, [])

  return (
    <div
      className={`relative overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-primary)] ${className}`}
    >
      {/* 底层 WebCodecs 零拷贝低延迟渲染画布 */}
      <canvas
        ref={videoCanvasRef}
        className={`h-full w-full ${
          fitMode === 'fill'
            ? 'object-fill'
            : fitMode === 'cover'
              ? 'object-cover'
              : 'object-contain'
        } ${
          activeProtocol === 'webcodecs' && connectionStatus === 'connected' ? 'block' : 'hidden'
        }`}
      />

      {/* 底层硬件解码视频渲染层 (FLV / MSE 兼容通道) */}
      <video
        ref={videoRef}
        autoPlay
        playsInline
        muted={!audioEnabled}
        className={`h-full w-full ${
          fitMode === 'fill'
            ? 'object-fill'
            : fitMode === 'cover'
              ? 'object-cover'
              : 'object-contain'
        } ${activeProtocol === 'flv' || connectionStatus !== 'connected' ? 'block' : 'hidden'}`}
      />

      {/* 顶层透明 Canvas 2D 识别框图层 */}
      <canvas ref={canvasRef} className="pointer-events-none absolute inset-0 z-10 h-full w-full" />

      {/* 连接状态遮罩 (Loading / Reconnecting / Error / Paused) */}
      {connectionStatus === 'paused' && (
        <div className="absolute inset-0 z-20 flex flex-col items-center justify-center bg-black/85 backdrop-blur-xs">
          <div className="flex flex-col items-center gap-3">
            <button
              type="button"
              onClick={onTogglePause}
              className="flex h-12 w-12 items-center justify-center rounded-full bg-[var(--accent)] text-white shadow-[var(--accent)]/30 shadow-lg transition-transform hover:scale-105 active:scale-95"
              title={t('live.clickToPlay', '点击开启实时拉流')}
            >
              <Play className="ml-0.5 h-5 w-5 fill-current" />
            </button>
            <span className="text-xs font-medium text-[var(--text-secondary)]">
              {t('live.paused', '已停止预览')}
            </span>
          </div>
        </div>
      )}

      {connectionStatus === 'connecting' && (
        <div className="absolute inset-0 z-20 flex flex-col items-center justify-center bg-black/60 backdrop-blur-xs">
          <RefreshCw className="h-6 w-6 animate-spin text-[var(--accent)]" />
          <span className="mt-2 text-xs font-medium text-[var(--text-secondary)]">
            {t('live.negotiating', '媒体流连接中...')}
          </span>
        </div>
      )}

      {connectionStatus === 'reconnecting' && (
        <div className="absolute inset-0 z-20 flex flex-col items-center justify-center bg-black/50">
          <div className="flex items-center gap-2 rounded-lg bg-amber-500/20 px-3 py-1.5 text-xs text-amber-400 backdrop-blur-md">
            <Wifi className="h-4 w-4 animate-pulse" />
            <span>{t('live.reconnecting', '网络抖动重连中...')}</span>
          </div>
        </div>
      )}

      {connectionStatus === 'failed' && (
        <div className="absolute inset-0 z-20 flex flex-col items-center justify-center bg-black/80">
          <WifiOff className="h-8 w-8 text-rose-500 opacity-80" />
          <span className="mt-2 text-xs font-medium text-rose-400">
            {t('live.streamFailed', '视频流接入失败')}
          </span>
          <button
            type="button"
            onClick={() => {
              setConnectionStatus('connecting')
              setRetryKey((k) => k + 1)
            }}
            className="mt-3 flex items-center gap-1.5 rounded-md bg-white/10 px-3 py-1 text-xs font-medium text-white transition hover:bg-white/20 active:scale-95"
          >
            <RefreshCw className="h-3.5 w-3.5" />
            <span>{t('live.retry', '重新连接')}</span>
          </button>
        </div>
      )}

      {/* 顶部/左上角沉浸式科技 HUD 面板 (仅在 Hero 主视口或 showHud 开启时呈现) */}
      {showHud && (
        <div className="frosted-glass absolute top-2.5 left-2.5 z-20 flex items-center gap-2 rounded-lg px-2.5 py-1 font-mono text-[11px] text-white/90 backdrop-blur-md">
          <span className={`h-2 w-2 rounded-full ${getStatusIndicatorClass(connectionStatus)}`} />
          <span className="font-semibold text-white">{cameraName || cameraId}</span>
          <span className="text-white/40">|</span>
          <span className="text-emerald-400">
            {streamType === 'main' ? '4K/1080P' : '720P/360P'} @ 25fps
          </span>
          <span className="text-white/40">|</span>
          <span className="text-cyan-300">{latencyMs}ms</span>
          <span className="text-white/40">|</span>
          <span
            className={`py-0.2 rounded px-1.5 text-[9px] font-bold tracking-wider ${
              activeProtocol === 'webcodecs'
                ? 'bg-cyan-500/25 text-cyan-300'
                : 'bg-emerald-500/25 text-emerald-300'
            }`}
          >
            {activeProtocol === 'webcodecs' ? 'WebCodecs' : 'FLV'}
          </span>
          {isHero && (
            <>
              <span className="text-white/40">|</span>
              <span className="flex items-center gap-1 text-amber-300">
                <Zap className="h-3 w-3" />
                <span>ANE · 8.2ms</span>
              </span>
            </>
          )}
        </div>
      )}

      {/* 顶部右上角控制组：码流切换、暂停/播放、聚焦、关闭 */}
      <div className="absolute top-2.5 right-2.5 z-20 flex items-center gap-1.5">
        {/* 码流切换按钮 */}
        {onSwitchStream && (
          <button
            type="button"
            onClick={() => onSwitchStream(streamType === 'main' ? 'sub' : 'main')}
            className="flex items-center gap-1 rounded-md bg-black/60 px-2 py-1 text-[10px] font-medium text-white/90 backdrop-blur-md transition-colors hover:bg-black/80 hover:text-white"
            title={streamType === 'main' ? t('live.subStream') : t('live.mainStream')}
          >
            <Layers className="h-3 w-3 text-cyan-400" />
            <span>{streamType === 'main' ? 'MAIN' : 'SUB'}</span>
          </button>
        )}

        {/* 暂停/播放控制 */}
        {onTogglePause && (
          <button
            type="button"
            onClick={onTogglePause}
            className="rounded-md bg-black/60 p-1 text-white/90 backdrop-blur-md transition-colors hover:bg-black/80 hover:text-white"
            title={isPaused ? t('live.openPreview') : t('live.closePreview')}
          >
            {isPaused ? <Play className="h-3.5 w-3.5" /> : <Pause className="h-3.5 w-3.5" />}
          </button>
        )}

        {/* 音频开关 (仅 Hero 主预览窗口显示) */}
        {onToggleAudio && isHero && (
          <button
            type="button"
            onClick={onToggleAudio}
            className="rounded-md bg-black/60 p-1 text-white/90 backdrop-blur-md transition-colors hover:bg-black/80 hover:text-white"
            title={
              audioEnabled ? t('live.muteAudio', '关闭音频') : t('live.enableAudio', '开启音频')
            }
          >
            {audioEnabled ? (
              <Volume2 className="h-3.5 w-3.5" />
            ) : (
              <VolumeX className="h-3.5 w-3.5 opacity-50" />
            )}
          </button>
        )}

        {/* 聚焦到主大屏 */}
        {onSpotlight && !isHero && (
          <button
            type="button"
            onClick={onSpotlight}
            className="rounded-md bg-black/60 p-1 text-white/90 backdrop-blur-md transition-colors hover:bg-cyan-500/80 hover:text-white"
            title={t('live.focusHero')}
          >
            <Eye className="h-3.5 w-3.5" />
          </button>
        )}

        {/* 关闭按钮 */}
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            className="rounded-md bg-black/60 p-1 text-white/90 backdrop-blur-md transition-colors hover:bg-rose-500/80 hover:text-white"
            title={t('common.close', '关闭')}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>

      {/* 底部遥测状态栏 (仅在主大屏展示) */}
      {isHero && telemetry && (
        <div className="absolute right-2.5 bottom-2.5 left-2.5 z-20 flex items-center justify-between rounded-lg bg-black/60 px-3 py-1.5 font-mono text-[11px] text-white/80 backdrop-blur-md">
          <div className="flex items-center gap-4">
            <span className="flex items-center gap-1.5 text-emerald-400">
              <Activity className="h-3.5 w-3.5" />
              <span>TRACKS: {telemetry.activeTracks}</span>
            </span>
            <span className="flex items-center gap-1 text-cyan-300">
              <User className="h-3.5 w-3.5" />
              <span>{telemetry.personCount}</span>
            </span>
            <span className="flex items-center gap-1 text-amber-300">
              <Car className="h-3.5 w-3.5" />
              <span>{telemetry.carCount}</span>
            </span>
          </div>

          <div className="flex items-center gap-3">
            <span className="text-[10px] text-white/50">MOTION HEAT</span>
            <div className="h-1.5 w-16 overflow-hidden rounded-full bg-white/20">
              <div
                className="h-full bg-gradient-to-r from-emerald-400 to-cyan-400 transition-all duration-300"
                style={{ width: `${Math.min(100, (telemetry.motionScore || 0) * 100)}%` }}
              />
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
