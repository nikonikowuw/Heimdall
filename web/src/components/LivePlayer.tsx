import { memo, useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import {
  Activity,
  Camera,
  Car,
  Check,
  Eye,
  EyeOff,
  Layers,
  Maximize,
  Maximize2,
  Minimize,
  Minimize2,
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
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import mpegts from 'mpegts.js'
import { useTranslation } from 'react-i18next'
import { cameraApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import { telemetryStore } from '@/lib/telemetryStore'
import { trackStore } from '@/lib/trackStore'
import { getVideoContentRect } from '@/lib/videoContentRect'
import { findSupportedCodecProfile, WebCodecsPlayer } from '@/lib/webcodecs'
import { useAuthStore } from '@/stores/auth'
import type { CameraTelemetry, TrackedBBox } from '@/types'

type ConnectionStatus =
  'connecting' | 'connected' | 'reconnecting' | 'failed' | 'paused' | 'standby'
type LatencyKind = 'renderLag' | 'bufferLag'

function getStatusIndicatorClass(status: ConnectionStatus): string {
  // 连接态只用颜色区分，不启用循环脉冲/闪烁。
  // 实测此前本页 5 个 6～12px 状态点在 1920×1080@2x 下合计占用约 9%。
  switch (status) {
    case 'connected':
      return 'bg-emerald-400'
    case 'reconnecting':
      return 'bg-amber-400'
    case 'connecting':
      return 'bg-cyan-400'
    case 'paused':
      return 'bg-amber-400'
    case 'failed':
      return 'bg-[var(--status-danger)]'
    case 'standby':
      return 'bg-slate-400'
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
  showOsd?: boolean
  onSpotlight?: () => void
  onClose?: () => void
  onTogglePause?: () => void
  onSwitchStream?: (stream: 'main' | 'sub') => void
  onToggleOsd?: () => void
  onToggleFitMode?: () => void
  onLatencyChange?: (latencyMs: number, kind: LatencyKind) => void
  onVideoSizeChange?: (width: number, height: number) => void
  onSnapshot?: (blob: Blob) => void
  /** 是否允许离开视口自动休眠拉流以节省边缘计算与网络资源 (默认仅辅流卡片生效) */
  enableAutoStandby?: boolean
  /** 当前目标码流编码；用于判断是否需要 WebCodecs 视频 + audio-only FLV 双通道 */
  videoCodec?: string
  /** 是否启用音频输出；H.264 使用音视频 FLV，H.265 在 MSE 不支持时使用独立 AAC 通道 */
  audioEnabled?: boolean
  onToggleAudio?: () => void
}

const VIDEO_MEDIA_EVENTS = [
  'loadstart',
  'loadedmetadata',
  'loadeddata',
  'canplay',
  'resize',
  'play',
  'playing',
  'timeupdate',
] as const

/**
 * 工业级抗抖动平滑 mpegts.js 播放器配置
 *
 * 核心设计原则：
 * 1. 禁用暴力追帧 (liveBufferLatencyChasing: false) 与过浅缓冲区，绝不在网络抖动时反复修改播放倍速导致画面卡顿；
 * 2. 维持 1.0s 稳定抗抖动 JitterBuffer 与 2.5s 弹性窗口，保证在 1~2s 安防长 GOP 下画面丝滑流畅、零卡顿；
 * 3. 画面呈现与检测框的时空同步通过 Canvas 离屏时间戳逆向对齐解决，不再依赖破坏性追帧。
 */
const SMOOTH_MPEGTS_CONFIG: mpegts.Config = {
  enableWorker: false,
  lazyLoad: false,
  enableStashBuffer: false,
  stashInitialSize: 64,
  liveBufferLatencyChasing: false,
  liveSync: true,
  liveSyncMaxLatency: 2.5,
  liveSyncTargetLatency: 1.0,
  autoCleanupSourceBuffer: true,
  autoCleanupMaxBackwardDuration: 10,
  autoCleanupMinBackwardDuration: 5,
}

/** FLV/MSE 播放模式下视频画面呈现与 AI 航迹的匹配容差时间 (毫秒) */
const FLV_TRACK_PTS_TOLERANCE_MS = 350

/** FLV 画面缓冲滞后低通滤波的一阶时间常数 (毫秒) */
const LAG_SMOOTHING_TIME_CONSTANT_MS = 300

/** FLV 画面平滑缓冲滞后初始默认值 (毫秒，对齐 liveSyncTargetLatency: 1.0s) */
const DEFAULT_SMOOTH_LAG_MS = 1000

interface EstimateVideoPtsParams {
  protocol: 'webcodecs' | 'flv'
  wcPlayer: WebCodecsPlayer | null
  videoEl: HTMLVideoElement | null
  cameraId: string
  smoothLagMsRef: { current: number }
  lastLagSampleTimeRef: { current: number }
}

/**
 * 动态推导当前画面呈现时刻的源帧绝对 PTS (毫秒)
 */
function estimateVideoPts({
  protocol,
  wcPlayer,
  videoEl,
  cameraId,
  smoothLagMsRef,
  lastLagSampleTimeRef,
}: EstimateVideoPtsParams): number | null {
  if (protocol === 'webcodecs') {
    return wcPlayer?.getCurrentPts() ?? null
  }

  if (!videoEl || videoEl.paused || videoEl.readyState < 2 || videoEl.buffered.length === 0) {
    return null
  }

  const bufferedEnd = videoEl.buffered.end(videoEl.buffered.length - 1)
  const instantLagMs = Math.max(0, (bufferedEnd - videoEl.currentTime) * 1000)

  // 基于时间步长的自适应一阶低通滤波 (EMA)：消除 60Hz/120Hz 高刷屏收敛速率差异与分包阶梯抖动
  const now = performance.now()
  const lastSample = lastLagSampleTimeRef.current
  const dt = lastSample > 0 ? Math.min(100, Math.max(1, now - lastSample)) : 16.6
  lastLagSampleTimeRef.current = now

  const alpha = 1 - Math.exp(-dt / LAG_SMOOTHING_TIME_CONSTANT_MS)
  const smoothLag = smoothLagMsRef.current * (1 - alpha) + instantLagMs * alpha
  smoothLagMsRef.current = smoothLag

  const latestPts = trackStore.getLatestTrackPts(cameraId)
  return latestPts != null && latestPts > 0 ? Math.round(latestPts - smoothLag) : null
}

export const LivePlayer = memo(function LivePlayer({
  cameraId,
  cameraName,
  className = '',
  showHud = true,
  isHero = false,
  isPaused = false,
  stream,
  telemetry,
  trackedObjects,
  fitMode: propFitMode,
  showOsd: propShowOsd,
  videoCodec,
  onSpotlight,
  onClose,
  onTogglePause,
  onSwitchStream,
  onToggleOsd,
  onToggleFitMode,
  onLatencyChange,
  onVideoSizeChange,
  onSnapshot,
  enableAutoStandby = true,
  audioEnabled,
  onToggleAudio,
}: LivePlayerProps) {
  const { t } = useTranslation('camera')
  const reducedMotion = useReducedMotion()
  const streamType = stream || (isHero ? 'main' : 'sub')

  const containerRef = useRef<HTMLDivElement>(null)
  const videoRef = useRef<HTMLVideoElement>(null)
  const audioRef = useRef<HTMLAudioElement>(null)
  const videoCanvasRef = useRef<HTMLCanvasElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const wcPlayerRef = useRef<WebCodecsPlayer | null>(null)
  const activeProtocolRef = useRef<'webcodecs' | 'flv'>('flv')
  const externalTracksRef = useRef<TrackedBBox[] | undefined>(trackedObjects)
  const smoothLagMsRef = useRef<number>(DEFAULT_SMOOTH_LAG_MS)
  const lastLagSampleTimeRef = useRef<number>(0)

  // 视口与全屏控制
  const [isFullscreen, setIsFullscreen] = useState<boolean>(false)
  const [isControlsVisible, setIsControlsVisible] = useState<boolean>(true)
  const [isControlsFocused, setIsControlsFocused] = useState<boolean>(false)
  const idleTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [isInView, setIsInView] = useState<boolean>(!enableAutoStandby || isHero)

  // 快照与微动效
  const [isFlashing, setIsFlashing] = useState<boolean>(false)
  const [snapshotBannerVisible, setSnapshotBannerVisible] = useState<boolean>(false)

  // 画面显示模式 (内部自管或受控)
  const [internalFitMode, setInternalFitMode] = useState<'contain' | 'cover'>(
    propFitMode === 'cover' ? 'cover' : 'contain',
  )
  const currentFitMode = propFitMode ?? internalFitMode

  // AI 标注图层开关 (受控或非受控)
  const [internalShowOsd, setInternalShowOsd] = useState<boolean>(true)
  const currentShowOsd = propShowOsd !== undefined ? propShowOsd : internalShowOsd

  const telemetrySubscriptionCameraId = isHero ? cameraId : ''
  const subscribedTelemetry = useSyncExternalStore(
    useCallback(
      (onStoreChange) => telemetryStore.subscribe(telemetrySubscriptionCameraId, onStoreChange),
      [telemetrySubscriptionCameraId],
    ),
    useCallback(
      () => telemetryStore.getTelemetry(telemetrySubscriptionCameraId),
      [telemetrySubscriptionCameraId],
    ),
    () => undefined,
  )
  const displayTelemetry = telemetry ?? (isHero ? subscribedTelemetry : undefined)

  const [internalAudioEnabled, setInternalAudioEnabled] = useState<boolean>(false)
  const isAudioActive = audioEnabled !== undefined ? audioEnabled : internalAudioEnabled

  const handleToggleAudio = () => {
    if (onToggleAudio) {
      onToggleAudio()
    } else {
      setInternalAudioEnabled((prev) => !prev)
    }
  }

  // 外部显式传入目标检测框时同步至 ref，零 React 重排与零 RAF 重启开销
  useEffect(() => {
    externalTracksRef.current = trackedObjects
  }, [trackedObjects])

  const preferredVideoCodec: 'h264' | 'h265' = /h\.?265|hevc/i.test(videoCodec ?? '')
    ? 'h265'
    : 'h264'

  const effectivePaused = isPaused || (!isHero && enableAutoStandby && !isInView)
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>(
    effectivePaused ? 'standby' : 'connecting',
  )
  const [activeProtocol, setActiveProtocol] = useState<'webcodecs' | 'flv'>('flv')
  activeProtocolRef.current = activeProtocol
  const [latencyMs, setLatencyMs] = useState<number | null>(null)
  const [latencyKind, setLatencyKind] = useState<LatencyKind>('renderLag')
  const [sourceSize, setSourceSize] = useState<{ width: number; height: number } | null>(null)
  const [retryKey, setRetryKey] = useState<number>(0)
  const autoRetryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const retryAttemptRef = useRef<number>(0)
  const stableTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const audioPlayerRef = useRef<mpegts.Player | null>(null)
  const isAudioActiveRef = useRef(isAudioActive)
  isAudioActiveRef.current = isAudioActive

  // 保持外部回调引用最新，避免将其作为流重连依赖项导致重连风暴
  const onLatencyChangeRef = useRef(onLatencyChange)
  onLatencyChangeRef.current = onLatencyChange
  const onVideoSizeChangeRef = useRef(onVideoSizeChange)
  onVideoSizeChangeRef.current = onVideoSizeChange
  const lastReportedVideoSizeRef = useRef<{ width: number; height: number } | null>(null)

  const reportVideoSize = useCallback((width: number, height: number) => {
    if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) return
    const previous = lastReportedVideoSizeRef.current
    if (previous?.width === width && previous.height === height) return
    lastReportedVideoSizeRef.current = { width, height }
    setSourceSize({ width, height })
    onVideoSizeChangeRef.current?.(width, height)
  }, [])

  // 当前会话中 WebCodecs 是否不可用或已降级（避免在 WebSocket 连接失败后进入重连死循环）
  const webCodecsFailedRef = useRef<boolean>(false)
  useEffect(() => {
    webCodecsFailedRef.current = false
    lastLatencyReportTimeRef.current = 0
    lastReportedVideoSizeRef.current = null
    setLatencyMs(null)
    setSourceSize(null)
  }, [cameraId, streamType])

  // 延迟遥测上报时间节流（限频 800ms，防高频 React 状态重绘与外部回调颠簸；首帧无延迟立即上报）
  const lastLatencyReportTimeRef = useRef<number>(0)
  const reportLatency = useCallback((lat: number, kind: LatencyKind) => {
    const now = performance.now()
    if (lastLatencyReportTimeRef.current === 0 || now - lastLatencyReportTimeRef.current >= 800) {
      lastLatencyReportTimeRef.current = now
      setLatencyMs(lat)
      setLatencyKind(kind)
      onLatencyChangeRef.current?.(lat, kind)
    }
  }, [])

  useEffect(
    () => () => {
      if (idleTimerRef.current) clearTimeout(idleTimerRef.current)
    },
    [],
  )

  // 视口可见性检测 (非 Hero 辅流在移出视口时休眠)
  useEffect(() => {
    if (isHero || !enableAutoStandby) {
      setIsInView(true)
      return
    }

    const el = containerRef.current
    if (!el || typeof IntersectionObserver === 'undefined') {
      setIsInView(true)
      return
    }

    const observer = new IntersectionObserver(
      ([entry]) => {
        setIsInView(entry.isIntersecting)
      },
      { rootMargin: '120px 0px 120px 0px', threshold: 0.05 },
    )
    observer.observe(el)
    return () => observer.disconnect()
  }, [isHero, enableAutoStandby])

  // 全屏状态监听
  useEffect(() => {
    const handleFsChange = () => {
      setIsFullscreen(document.fullscreenElement === containerRef.current)
    }
    document.addEventListener('fullscreenchange', handleFsChange)
    return () => document.removeEventListener('fullscreenchange', handleFsChange)
  }, [])

  const handlePointerMove = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      setIsControlsVisible(true)
      if (idleTimerRef.current) clearTimeout(idleTimerRef.current)
      if (!isHero || event.pointerType === 'touch') return
      idleTimerRef.current = setTimeout(() => {
        if (!effectivePaused && connectionStatus === 'connected' && !isControlsFocused) {
          setIsControlsVisible(false)
        }
      }, 3500)
    },
    [isHero, effectivePaused, connectionStatus, isControlsFocused],
  )

  const handlePointerLeave = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (idleTimerRef.current) clearTimeout(idleTimerRef.current)
      if (event.pointerType === 'touch') return
      if (isHero && !effectivePaused && connectionStatus === 'connected' && !isControlsFocused) {
        setIsControlsVisible(false)
      }
    },
    [isHero, effectivePaused, connectionStatus, isControlsFocused],
  )

  const handleFocusCapture = useCallback(() => {
    setIsControlsFocused(true)
    setIsControlsVisible(true)
  }, [])

  const handleBlurCapture = useCallback((event: React.FocusEvent<HTMLDivElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
      setIsControlsFocused(false)
    }
  }, [])

  const controlsVisible = isControlsVisible || isControlsFocused || connectionStatus !== 'connected'
  let displayedLatencyKind = latencyKind
  if (latencyMs === null) {
    displayedLatencyKind = activeProtocol === 'webcodecs' ? 'renderLag' : 'bufferLag'
  }
  const latencyLabelKey = displayedLatencyKind === 'bufferLag' ? 'live.bufferLag' : 'live.renderLag'
  const latencyShortLabelKey =
    displayedLatencyKind === 'bufferLag' ? 'live.bufferLagShort' : 'live.renderLagShort'

  const handleToggleFullscreen = useCallback(() => {
    const el = containerRef.current
    if (!el) return
    if (document.fullscreenElement) {
      void document.exitFullscreen().catch(() => {})
    } else {
      void el.requestFullscreen().catch(() => {})
    }
  }, [])

  const handleToggleFitModeInternal = useCallback(() => {
    if (onToggleFitMode) {
      onToggleFitMode()
    } else {
      setInternalFitMode((prev) => (prev === 'contain' ? 'cover' : 'contain'))
    }
  }, [onToggleFitMode])

  const handleToggleOsdInternal = useCallback(() => {
    if (onToggleOsd) {
      onToggleOsd()
    } else {
      setInternalShowOsd((prev) => !prev)
    }
  }, [onToggleOsd])

  // 快照抓拍导出
  const handleCaptureSnapshot = useCallback(() => {
    setIsFlashing(true)
    setTimeout(() => setIsFlashing(false), 200)

    const downloadBlob = (blob: Blob) => {
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = `snapshot_${cameraId}_${Date.now()}.png`
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      URL.revokeObjectURL(url)

      setSnapshotBannerVisible(true)
      setTimeout(() => setSnapshotBannerVisible(false), 2500)
      onSnapshot?.(blob)
    }

    if (activeProtocolRef.current === 'webcodecs' && videoCanvasRef.current) {
      videoCanvasRef.current.toBlob((blob) => {
        if (blob) downloadBlob(blob)
      }, 'image/png')
      return
    }

    const videoEl = videoRef.current
    if (videoEl && videoEl.videoWidth > 0 && videoEl.videoHeight > 0) {
      try {
        const offscreen = document.createElement('canvas')
        offscreen.width = videoEl.videoWidth
        offscreen.height = videoEl.videoHeight
        const ctx = offscreen.getContext('2d')
        if (ctx) {
          ctx.drawImage(videoEl, 0, 0, offscreen.width, offscreen.height)
          offscreen.toBlob((blob) => {
            if (blob) downloadBlob(blob)
          }, 'image/png')
        }
      } catch {
        // 捕获可能由非同源视频引起的静默异常
      }
    }
  }, [cameraId, onSnapshot])

  const destroyAudioPlayer = useCallback(() => {
    if (audioPlayerRef.current) {
      try {
        audioPlayerRef.current.pause()
        audioPlayerRef.current.unload()
        audioPlayerRef.current.detachMediaElement()
        audioPlayerRef.current.destroy()
      } catch {
        // ignore teardown errors
      }
      audioPlayerRef.current = null
    }
    const audioEl = audioRef.current
    if (audioEl) {
      try {
        audioEl.pause()
        audioEl.removeAttribute('src')
        audioEl.load()
      } catch {
        // ignore cleanup errors
      }
    }
  }, [])

  const startAudioOnlyPlayer = useCallback(() => {
    destroyAudioPlayer()
    const audioEl = audioRef.current
    if (!audioEl || !isAudioActiveRef.current || !mpegts.isSupported()) return

    const audioUrl = cameraApi.getLiveStreamUrl(cameraId, streamType, true, false)
    mpegts.LoggingControl.enableAll = false

    try {
      audioEl.muted = false
      audioEl.volume = 1.0
      const audioPlayer = mpegts.createPlayer(
        {
          type: 'flv',
          isLive: true,
          url: audioUrl,
          hasAudio: true,
          hasVideo: false,
          cors: true,
        },
        SMOOTH_MPEGTS_CONFIG,
      )
      audioPlayerRef.current = audioPlayer
      audioPlayer.attachMediaElement(audioEl)
      audioPlayer.load()
      const playPromise = audioPlayer.play()
      if (playPromise && typeof playPromise.catch === 'function') {
        playPromise.catch(() => {
          if (audioEl) {
            audioEl.muted = true
            const retryPlay = audioPlayer.play()
            if (retryPlay && typeof retryPlay.catch === 'function') {
              void retryPlay.catch(() => undefined)
            }
          }
        })
      }
      audioPlayer.on(mpegts.Events.ERROR, (_type: string, detail: string, info: unknown) => {
        const httpCode = (info as { code?: number })?.code
        if (detail === 'HttpStatusCodeInvalid' && httpCode === 401) {
          useAuthStore.getState().logout()
        }
      })
    } catch {
      destroyAudioPlayer()
    }
  }, [cameraId, streamType, destroyAudioPlayer])

  useEffect(() => {
    if (effectivePaused) {
      destroyAudioPlayer()
      return
    }
    if (isAudioActive) {
      startAudioOnlyPlayer()
    } else {
      destroyAudioPlayer()
    }
  }, [isAudioActive, effectivePaused, startAudioOnlyPlayer, destroyAudioPlayer])

  useEffect(() => {
    if (effectivePaused) {
      setConnectionStatus(isPaused ? 'paused' : 'standby')
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
      if (stableTimerRef.current) {
        clearTimeout(stableTimerRef.current)
        stableTimerRef.current = null
      }

      const attempt = retryAttemptRef.current
      const baseMs = Math.min(1000 * 2 ** attempt, 30_000)
      const jitterMs = baseMs * 0.1 * Math.random()
      const delayMs = Math.round(baseMs + jitterMs)
      retryAttemptRef.current = Math.min(attempt + 1, 6)

      autoRetryTimerRef.current = setTimeout(() => {
        if (!isCancelled) {
          setRetryKey((k) => k + 1)
        }
      }, delayMs)
    }

    function canUseMseVideo() {
      if (!mpegts.isSupported()) return false
      if (preferredVideoCodec === 'h265') {
        try {
          return mpegts.getFeatureList().mseH265Playback
        } catch {
          return false
        }
      }
      return true
    }

    const handlePlaying = () => {
      if (!isCancelled) {
        setConnectionStatus('connected')
        if (videoEl && videoEl.buffered.length > 0) {
          const bufferedEnd = videoEl.buffered.end(videoEl.buffered.length - 1)
          const latency = Math.max(0, Math.round((bufferedEnd - videoEl.currentTime) * 1000))
          reportLatency(latency, 'bufferLag')
          smoothLagMsRef.current = latency
          lastLagSampleTimeRef.current = performance.now()
        }
        if (!stableTimerRef.current) {
          stableTimerRef.current = setTimeout(() => {
            retryAttemptRef.current = 0
            stableTimerRef.current = null
          }, 5000)
        }
      }
    }

    function startFlvPlayer() {
      if (isCancelled || !videoEl || !cameraId) return
      setActiveProtocol('flv')
      setConnectionStatus('connecting')

      const flvUrl = cameraApi.getLiveStreamUrl(cameraId, streamType, false, true)

      if (!canUseMseVideo()) {
        setConnectionStatus('failed')
        return
      }

      mpegts.LoggingControl.enableAll = false

      try {
        videoEl.muted = true

        flvPlayer = mpegts.createPlayer(
          {
            type: 'flv',
            isLive: true,
            url: flvUrl,
            hasAudio: false,
            hasVideo: true,
            cors: true,
          },
          SMOOTH_MPEGTS_CONFIG,
        )

        flvPlayer.attachMediaElement(videoEl)
        flvPlayer.load()

        const playPromise = flvPlayer.play()
        if (playPromise && typeof playPromise.catch === 'function') {
          playPromise.catch(() => {
            if (videoEl && isAudioActiveRef.current && !isCancelled) {
              videoEl.muted = true
              const retryPlay = flvPlayer?.play()
              if (retryPlay && typeof retryPlay.catch === 'function') {
                void retryPlay.catch(() => undefined)
              }
            }
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

    async function startWebCodecsPlayer(preferredCodecMime?: string) {
      if (!videoCanvas || isCancelled) return false

      try {
        const wsUrl = cameraApi.getWebCodecsWsUrl(cameraId, streamType)
        let hasPlayed = false

        wcPlayer = new WebCodecsPlayer({
          wsUrl,
          canvas: videoCanvas,
          preferredCodec: preferredVideoCodec,
          preferredCodecMime,
          onVideoSizeChange: reportVideoSize,
          onPlaying: (lat) => {
            if (isCancelled) return
            hasPlayed = true
            setActiveProtocol('webcodecs')
            setConnectionStatus('connected')
            reportLatency(lat, 'renderLag')
            if (!stableTimerRef.current) {
              stableTimerRef.current = setTimeout(() => {
                retryAttemptRef.current = 0
                stableTimerRef.current = null
              }, 5000)
            }
          },
          onError: () => {
            if (isCancelled) return
            if (wcPlayer) {
              wcPlayer.destroy()
              wcPlayer = null
            }
            wcPlayerRef.current = null
            webCodecsFailedRef.current = true
            if (canUseMseVideo()) {
              startFlvPlayer()
            } else {
              setConnectionStatus('failed')
            }
          },
          onClose: () => {
            if (isCancelled) return
            // 已被降级处理，避免重复触发重连
            if (webCodecsFailedRef.current) return

            // 若从未收到数据帧且未成功播放，判定当前 WebSocket 连接不可达，平滑降级至 FLV
            if (!hasPlayed) {
              if (wcPlayer) {
                wcPlayer.destroy()
                wcPlayer = null
              }
              wcPlayerRef.current = null
              webCodecsFailedRef.current = true
              if (canUseMseVideo()) {
                startFlvPlayer()
                return
              }
            }

            setConnectionStatus('reconnecting')
            scheduleRetry()
          },
        })
        wcPlayerRef.current = wcPlayer
        return true
      } catch {
        webCodecsFailedRef.current = true
        return false
      }
    }

    async function startPlayer() {
      if (!cameraId) return
      setConnectionStatus('connecting')

      if (!webCodecsFailedRef.current) {
        const supportedMime = await findSupportedCodecProfile(preferredVideoCodec)

        if (supportedMime && !isCancelled) {
          try {
            const selected = await startWebCodecsPlayer(supportedMime)
            if (selected) {
              return
            }
          } catch {
            webCodecsFailedRef.current = true
          }
        }
      }

      startFlvPlayer()
    }

    const handleVideoEvent = (e: Event) => {
      if (
        videoEl &&
        (e.type === 'loadedmetadata' ||
          e.type === 'loadeddata' ||
          e.type === 'canplay' ||
          e.type === 'resize')
      ) {
        reportVideoSize(videoEl.videoWidth, videoEl.videoHeight)
      }
      if (e.type === 'playing' || e.type === 'loadeddata' || e.type === 'timeupdate') {
        handlePlaying()
      }
    }

    if (videoEl) {
      videoEl.muted = true
      VIDEO_MEDIA_EVENTS.forEach((evt) => videoEl.addEventListener(evt, handleVideoEvent))
    }

    startPlayer()

    return () => {
      isCancelled = true
      if (autoRetryTimerRef.current) {
        clearTimeout(autoRetryTimerRef.current)
        autoRetryTimerRef.current = null
      }
      if (stableTimerRef.current) {
        clearTimeout(stableTimerRef.current)
        stableTimerRef.current = null
      }
      if (videoEl) {
        VIDEO_MEDIA_EVENTS.forEach((evt) => videoEl.removeEventListener(evt, handleVideoEvent))
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
      smoothLagMsRef.current = DEFAULT_SMOOTH_LAG_MS
      lastLagSampleTimeRef.current = 0
    }
  }, [
    cameraId,
    streamType,
    effectivePaused,
    isPaused,
    retryKey,
    preferredVideoCodec,
    reportLatency,
    reportVideoSize,
  ])

  // Canvas 2D 离屏 60fps 绘制循环（包含 Retina 高分屏物理像素锐化）
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return

    if (effectivePaused || !currentShowOsd) {
      const ctx = canvas.getContext('2d')
      if (ctx) {
        ctx.clearRect(0, 0, canvas.width, canvas.height)
      }
      return
    }

    let animId: number
    let paintedLastFrame = false
    const render = () => {
      const ctx = canvas.getContext('2d')
      if (ctx) {
        const dpr = Math.max(1, window.devicePixelRatio || 1)
        const logicalW = canvas.width / dpr
        const logicalH = canvas.height / dpr
        const sourceWidth =
          activeProtocolRef.current === 'webcodecs'
            ? (videoCanvasRef.current?.width ?? 0)
            : (videoRef.current?.videoWidth ?? 0)
        const sourceHeight =
          activeProtocolRef.current === 'webcodecs'
            ? (videoCanvasRef.current?.height ?? 0)
            : (videoRef.current?.videoHeight ?? 0)
        const contentRect = getVideoContentRect(
          logicalW,
          logicalH,
          sourceWidth,
          sourceHeight,
          currentFitMode,
        )

        // 动态推导当前画面呈现时刻的源帧绝对 PTS (毫秒)
        const currentVideoPts = estimateVideoPts({
          protocol: activeProtocolRef.current,
          wcPlayer: wcPlayerRef.current,
          videoEl: videoRef.current,
          cameraId,
          smoothLagMsRef,
          lastLagSampleTimeRef,
        })

        if (!contentRect) {
          if (paintedLastFrame) {
            ctx.clearRect(0, 0, canvas.width, canvas.height)
            paintedLastFrame = false
          }
          animId = requestAnimationFrame(render)
          return
        }

        const tracks =
          externalTracksRef.current ??
          trackStore.getTracks(cameraId, currentVideoPts, FLV_TRACK_PTS_TOLERANCE_MS)

        // 空轨迹帧且画布已清空时不再清屏：避免每帧扫描整张高 DPR 画布。
        if (tracks.length === 0 && !paintedLastFrame) {
          animId = requestAnimationFrame(render)
          return
        }
        paintedLastFrame = tracks.length > 0

        ctx.clearRect(0, 0, canvas.width, canvas.height)
        ctx.save()
        ctx.scale(dpr, dpr)

        for (const item of tracks) {
          const [nx1, ny1, nx2, ny2] = item.bbox
          const x = contentRect.x + nx1 * contentRect.width
          const y = contentRect.y + ny1 * contentRect.height
          const boxW = (nx2 - nx1) * contentRect.width
          const boxH = (ny2 - ny1) * contentRect.height

          // 1. 绘制历史轨迹线条
          if (item.trajectory && item.trajectory.length > 1) {
            ctx.beginPath()
            const pts = item.trajectory
            ctx.moveTo(
              contentRect.x + pts[0][0] * contentRect.width,
              contentRect.y + pts[0][1] * contentRect.height,
            )
            for (let i = 1; i < pts.length; i++) {
              ctx.lineTo(
                contentRect.x + pts[i][0] * contentRect.width,
                contentRect.y + pts[i][1] * contentRect.height,
              )
            }
            ctx.strokeStyle = 'rgba(6, 182, 212, 0.45)'
            ctx.lineWidth = isHero ? 2.5 : 1.5
            ctx.stroke()
          }

          // 2. 绘制发光识别框
          const isFace = item.label.toLowerCase() === 'face'
          const isPerson = item.label.toLowerCase() === 'person'
          let strokeColor = '#10b981'
          if (isPerson) {
            strokeColor = '#06b6d4'
          } else if (isFace) {
            strokeColor = '#8b5cf6'
          }
          ctx.strokeStyle = strokeColor
          ctx.lineWidth = isHero ? 2 : 1.5
          ctx.strokeRect(x, y, boxW, boxH)

          // 2.1 结构化人脸框高亮
          if (item.face) {
            const [fx1, fy1, fx2, fy2] = item.face.bbox
            const fx = contentRect.x + fx1 * contentRect.width
            const fy = contentRect.y + fy1 * contentRect.height
            const fboxW = (fx2 - fx1) * contentRect.width
            const fboxH = (fy2 - fy1) * contentRect.height

            ctx.save()
            ctx.strokeStyle = '#a855f7'
            ctx.lineWidth = isHero ? 2 : 1.5
            ctx.setLineDash([3, 2])
            ctx.strokeRect(fx, fy, fboxW, fboxH)

            const faceConfidence = item.face.confidence
            const faceText =
              typeof faceConfidence === 'number' && Number.isFinite(faceConfidence)
                ? `Face ${(faceConfidence * 100).toFixed(0)}%`
                : 'Face'
            ctx.font = '600 9px monospace'
            const faceWidth = ctx.measureText(faceText).width
            ctx.fillStyle = 'rgba(168, 85, 247, 0.9)'
            ctx.fillRect(fx, Math.max(0, fy - 12), faceWidth + 6, 11)
            ctx.fillStyle = '#ffffff'
            ctx.fillText(faceText, fx + 3, Math.max(9, fy - 3))
            ctx.restore()
          }

          // 3. 绘制目标标签与置信度胶囊
          ctx.fillStyle = isPerson
            ? 'rgba(6, 182, 212, 0.85)'
            : isFace
              ? 'rgba(139, 92, 246, 0.85)'
              : 'rgba(16, 185, 129, 0.85)'
          const detPct = `${(item.confidence * 100).toFixed(0)}%`
          const faceQuality = item.face?.qualityScore ?? (isFace ? item.qualityScore : undefined)
          const labelText =
            faceQuality !== undefined
              ? `#${item.trackId} ${item.label} Q:${(faceQuality * 100).toFixed(0)}% (${detPct})`
              : `#${item.trackId} ${item.label} ${detPct}`
          ctx.font = isHero ? '600 11px monospace' : '500 9px monospace'
          const textWidth = ctx.measureText(labelText).width
          ctx.fillRect(x, Math.max(0, y - (isHero ? 18 : 14)), textWidth + 8, isHero ? 16 : 13)

          ctx.fillStyle = '#ffffff'
          ctx.fillText(labelText, x + 4, Math.max(10, y - (isHero ? 6 : 4)))
        }

        ctx.restore()
      }
      animId = requestAnimationFrame(render)
    }

    animId = requestAnimationFrame(render)
    return () => cancelAnimationFrame(animId)
  }, [cameraId, isHero, effectivePaused, currentShowOsd, currentFitMode])

  // 监听画布尺寸自适应并适配 DPR
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const dpr = Math.max(1, window.devicePixelRatio || 1)
        canvas.width = Math.round(entry.contentRect.width * dpr)
        canvas.height = Math.round(entry.contentRect.height * dpr)
      }
    })
    observer.observe(canvas)
    return () => observer.disconnect()
  }, [])

  return (
    <div
      ref={containerRef}
      onPointerMove={handlePointerMove}
      onPointerLeave={handlePointerLeave}
      onFocusCapture={handleFocusCapture}
      onBlurCapture={handleBlurCapture}
      className={`group on-dark-surface relative overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--video-surface)] ${className}`}
      role="group"
      aria-label={cameraName || cameraId}
    >
      {/* 抓拍快门瞬间白色闪光遮罩 */}
      {isFlashing && (
        <div className="animate-out fade-out pointer-events-none absolute inset-0 z-40 bg-white/70 duration-200" />
      )}
      {/* 抓拍成功微型横幅 (Motion AnimatePresence) */}
      <AnimatePresence mode="wait">
        {snapshotBannerVisible && (
          <motion.div
            key="snapshot-toast"
            initial={{ opacity: 0, y: reducedMotion ? 0 : 12, scale: reducedMotion ? 1 : 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: reducedMotion ? 0 : 8, scale: reducedMotion ? 1 : 0.95 }}
            transition={{
              duration: reducedMotion ? motionTokens.duration.fast : motionTokens.duration.normal,
              ease: motionTokens.easing.smooth,
            }}
            className="absolute bottom-4 left-1/2 z-30 flex -translate-x-1/2 items-center gap-2 rounded-full border border-emerald-500/40 bg-black/80 px-3.5 py-1 text-xs font-medium text-emerald-300 shadow-xl backdrop-blur-md"
          >
            <Check className="h-3.5 w-3.5 text-emerald-400" />
            <span>{t('live.snapshotSaved')}</span>
          </motion.div>
        )}
      </AnimatePresence>
      {/* 底层 WebCodecs 零拷贝低延迟渲染画布 */}
      <canvas
        ref={videoCanvasRef}
        className={`h-full w-full ${
          currentFitMode === 'fill'
            ? 'object-fill'
            : currentFitMode === 'cover'
              ? 'object-cover'
              : 'object-contain'
        } ${
          activeProtocol === 'webcodecs' && connectionStatus === 'connected' ? 'block' : 'hidden'
        }`}
      />
      {/* H.265 WebCodecs fallback 使用隐藏的独立 AAC FLV 音频元素 */}
      <audio ref={audioRef} autoPlay aria-hidden="true" className="hidden" />
      {/* 底层硬件解码视频渲染层 (FLV / MSE 兼容通道) */}
      <video
        ref={videoRef}
        autoPlay
        playsInline
        muted={!isAudioActive}
        className={`h-full w-full ${
          currentFitMode === 'fill'
            ? 'object-fill'
            : currentFitMode === 'cover'
              ? 'object-cover'
              : 'object-contain'
        } ${activeProtocol === 'flv' || connectionStatus !== 'connected' ? 'block' : 'hidden'}`}
      />
      {/* 顶层透明 Canvas 2D 识别框图层 */}
      <canvas
        ref={canvasRef}
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 z-10 h-full w-full"
      />
      {/* 状态遮罩层 (Motion AnimatePresence 丝滑状态切换) */}
      <AnimatePresence mode="wait">
        {connectionStatus === 'standby' && (
          <motion.div
            key="status-standby"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: motionTokens.duration.fast }}
            className="absolute inset-0 z-20 flex flex-col items-center justify-center bg-black/75 backdrop-blur-xs"
          >
            <div className="flex flex-col items-center gap-2 p-4 text-center">
              <div className="flex h-10 w-10 items-center justify-center rounded-full bg-white/5 text-slate-400">
                <Eye className="h-5 w-5 opacity-70" />
              </div>
              <span className="text-xs font-medium text-slate-300">
                {t('live.outOfViewStandby')}
              </span>
            </div>
          </motion.div>
        )}

        {connectionStatus === 'paused' && (
          <motion.div
            key="status-paused"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: motionTokens.duration.fast }}
            className="absolute inset-0 z-20 flex flex-col items-center justify-center bg-black/85 backdrop-blur-xs"
          >
            <div className="flex flex-col items-center gap-3">
              <button
                type="button"
                onClick={onTogglePause}
                className="flex h-12 w-12 items-center justify-center rounded-full bg-[var(--accent)] text-white shadow-lg transition-transform hover:scale-105 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none active:scale-95"
                aria-label={t('live.clickToPlay')}
                title={t('live.clickToPlay')}
              >
                <Play className="ml-0.5 h-5 w-5 fill-current" />
              </button>
              <span className="text-xs font-medium text-[var(--text-secondary)]">
                {t('live.paused')}
              </span>
            </div>
          </motion.div>
        )}

        {connectionStatus === 'connecting' && (
          <motion.div
            key="status-connecting"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: motionTokens.duration.fast }}
            className="absolute inset-0 z-20 flex flex-col items-center justify-center bg-black/60 backdrop-blur-xs"
          >
            <RefreshCw className="h-6 w-6 text-[var(--accent)]" />
            <span className="mt-2 text-xs font-medium text-[var(--text-secondary)]">
              {t('live.negotiating')}
            </span>
          </motion.div>
        )}

        {connectionStatus === 'reconnecting' && (
          <motion.div
            key="status-reconnecting"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: motionTokens.duration.fast }}
            className="absolute inset-0 z-20 flex flex-col items-center justify-center bg-black/50"
          >
            <div className="flex items-center gap-2 rounded-lg bg-amber-500/20 px-3 py-1.5 text-xs text-amber-400 backdrop-blur-md">
              <Wifi className="h-4 w-4" />
              <span>{t('live.reconnecting')}</span>
            </div>
          </motion.div>
        )}

        {connectionStatus === 'failed' && (
          <motion.div
            key="status-failed"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: motionTokens.duration.fast }}
            className="absolute inset-0 z-20 flex flex-col items-center justify-center bg-black/80"
          >
            <WifiOff className="h-8 w-8 text-[var(--status-danger)] opacity-80" />
            <span className="mt-2 text-xs font-medium text-[var(--status-danger)]">
              {t('live.streamFailed')}
            </span>
            <button
              type="button"
              onClick={() => {
                setConnectionStatus('connecting')
                setRetryKey((k) => k + 1)
              }}
              className="mt-3 flex min-h-11 items-center gap-1.5 rounded-md bg-white/10 px-3 py-1 text-xs font-medium text-white transition hover:bg-white/20 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none active:scale-95"
              aria-label={t('live.retry')}
            >
              <RefreshCw className="h-3.5 w-3.5" />
              <span>{t('live.retry')}</span>
            </button>
          </motion.div>
        )}
      </AnimatePresence>
      {/* 顶部/左上角沉浸式科技 HUD 面板 (专业视频 OSD 深色防眩光底盘，杜绝亮色模式下白底白字低对比度问题) */}
      {showHud && (
        <div
          // 刻意不用 backdrop-filter：本 HUD 悬浮于实时视频之上，视频帧变化时会重复采样，
          // 且 bg-black/75 下模糊效果几乎不可见。
          className={`absolute top-2.5 left-2.5 z-20 flex max-w-[calc(100%-1.25rem)] flex-wrap items-center gap-2 rounded-lg border border-white/15 bg-black/75 px-2.5 py-1 font-mono text-[11px] text-white/90 shadow-lg transition-opacity duration-300 ${
            controlsVisible ? 'opacity-100' : 'pointer-events-none opacity-0'
          }`}
        >
          <span className={`h-2 w-2 rounded-full ${getStatusIndicatorClass(connectionStatus)}`} />
          <span className="max-w-[120px] truncate font-semibold text-white">
            {cameraName || cameraId}
          </span>
          <span className="text-white/40">|</span>
          <span className="text-emerald-400">
            {sourceSize ? `${sourceSize.width}×${sourceSize.height}` : '--'}
          </span>
          <span className="text-white/40">|</span>
          <span className="text-cyan-300" title={t(latencyLabelKey)}>
            {t(latencyShortLabelKey)} {latencyMs === null ? '--' : `${latencyMs}ms`}
          </span>
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
          {isAudioActive && (
            <>
              <span className="text-white/40">|</span>
              <span className="flex items-center gap-1 text-emerald-400">
                <Volume2 className="h-3 w-3" />
                <span className="text-[9px] font-bold tracking-wider">AUDIO</span>
              </span>
            </>
          )}
          {isHero && (
            <>
              <span className="text-white/40">|</span>
              <span className="flex items-center gap-1 text-amber-300">
                <Zap className="h-3 w-3" />
                <span>NPU HW</span>
              </span>
            </>
          )}
        </div>
      )}
      {/* 顶部右上角控制组：抓拍、AI 标注切换、视口适应、码流、音频、全屏等 (支持闲置淡出) */}
      {showHud && (
        <div
          className={`absolute top-14 right-2.5 z-20 flex max-w-[calc(100%-1.25rem)] flex-wrap items-center justify-end gap-1.5 transition-opacity duration-300 sm:top-2.5 ${
            controlsVisible ? 'opacity-100' : 'pointer-events-none opacity-0'
          }`}
        >
          {/* 即时抓拍快照 */}
          <button
            type="button"
            onClick={handleCaptureSnapshot}
            className="min-h-11 min-w-11 rounded-md bg-black/60 p-1 text-white/90 transition-colors hover:bg-cyan-500/80 hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0 sm:min-w-0"
            aria-label={t('live.snapshot')}
            title={t('live.snapshot')}
          >
            <Camera className="h-3.5 w-3.5" />
          </button>

          {/* AI 标注图层切换 (OSD) */}
          <button
            type="button"
            onClick={handleToggleOsdInternal}
            aria-pressed={currentShowOsd}
            className={`min-h-11 min-w-11 rounded-md p-1 transition-colors focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0 sm:min-w-0 ${
              currentShowOsd
                ? 'bg-black/60 text-cyan-300 hover:bg-black/80 hover:text-white'
                : 'bg-black/60 text-white/40 hover:bg-black/80 hover:text-white'
            }`}
            aria-label={currentShowOsd ? t('live.toggleOsdShow') : t('live.toggleOsdHide')}
            title={currentShowOsd ? t('live.toggleOsdShow') : t('live.toggleOsdHide')}
          >
            {currentShowOsd ? <Eye className="h-3.5 w-3.5" /> : <EyeOff className="h-3.5 w-3.5" />}
          </button>

          {/* 画面适应模式切换 (Contain vs Cover) */}
          {isHero && (
            <button
              type="button"
              onClick={handleToggleFitModeInternal}
              className="min-h-11 min-w-11 rounded-md bg-black/60 p-1 text-white/90 transition-colors hover:bg-black/80 hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0 sm:min-w-0"
              aria-label={currentFitMode === 'contain' ? t('live.fitCover') : t('live.fitContain')}
              title={currentFitMode === 'contain' ? t('live.fitCover') : t('live.fitContain')}
            >
              {currentFitMode === 'contain' ? (
                <Maximize2 className="h-3.5 w-3.5" />
              ) : (
                <Minimize2 className="h-3.5 w-3.5" />
              )}
            </button>
          )}

          {/* 码流切换 */}
          {onSwitchStream && (
            <button
              type="button"
              onClick={() => onSwitchStream(streamType === 'main' ? 'sub' : 'main')}
              className="min-h-11 min-w-11 rounded-md bg-black/60 px-2 py-1 text-[10px] font-medium text-white/90 transition-colors hover:bg-black/80 hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0 sm:min-w-0"
              aria-label={streamType === 'main' ? t('live.subStream') : t('live.mainStream')}
              title={streamType === 'main' ? t('live.subStream') : t('live.mainStream')}
            >
              <Layers className="h-3 w-3 text-cyan-400" />
              <span>{streamType === 'main' ? 'MAIN' : 'SUB'}</span>
            </button>
          )}

          {/* 暂停/播放 */}
          {onTogglePause && (
            <button
              type="button"
              onClick={onTogglePause}
              className="min-h-11 min-w-11 rounded-md bg-black/60 p-1 text-white/90 transition-colors hover:bg-black/80 hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0 sm:min-w-0"
              aria-label={isPaused ? t('live.openPreview') : t('live.closePreview')}
              title={isPaused ? t('live.openPreview') : t('live.closePreview')}
            >
              {isPaused ? <Play className="h-3.5 w-3.5" /> : <Pause className="h-3.5 w-3.5" />}
            </button>
          )}

          {/* 音频切换 */}
          {(onToggleAudio || isHero || showHud) && (
            <button
              type="button"
              onClick={handleToggleAudio}
              aria-pressed={isAudioActive}
              className={`min-h-11 min-w-11 rounded-md p-1 transition-colors focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0 sm:min-w-0 ${
                isAudioActive
                  ? 'bg-[var(--accent)] text-white shadow-xs'
                  : 'bg-black/60 text-white/90 hover:bg-black/80 hover:text-white'
              }`}
              aria-label={isAudioActive ? t('live.muteAudio') : t('live.enableAudio')}
              title={isAudioActive ? t('live.muteAudio') : t('live.enableAudio')}
            >
              {isAudioActive ? (
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
              className="min-h-11 min-w-11 rounded-md bg-black/60 p-1 text-white/90 transition-colors hover:bg-cyan-500/80 hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0 sm:min-w-0"
              aria-label={t('live.focusHero')}
              title={t('live.focusHero')}
            >
              <Eye className="h-3.5 w-3.5" />
            </button>
          )}

          {/* 全屏切换 */}
          <button
            type="button"
            onClick={handleToggleFullscreen}
            className="min-h-11 min-w-11 rounded-md bg-black/60 p-1 text-white/90 transition-colors hover:bg-black/80 hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0 sm:min-w-0"
            aria-label={isFullscreen ? t('live.exitFullscreen') : t('live.fullscreen')}
            title={isFullscreen ? t('live.exitFullscreen') : t('live.fullscreen')}
          >
            {isFullscreen ? (
              <Minimize className="h-3.5 w-3.5" />
            ) : (
              <Maximize className="h-3.5 w-3.5" />
            )}
          </button>

          {/* 关闭按钮 */}
          {onClose && (
            <button
              type="button"
              onClick={onClose}
              className="min-h-11 min-w-11 rounded-md bg-black/60 p-1 text-white/90 transition-colors hover:bg-[var(--status-danger-solid)]/80 hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0 sm:min-w-0"
              aria-label={t('live.close')}
              title={t('live.close')}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      )}
      {/* 底部遥测状态栏 (仅在主大屏展示，支持闲置淡出) */}
      {isHero && displayTelemetry && (
        <div
          className={`absolute right-2.5 bottom-2.5 left-2.5 z-20 flex items-center justify-between rounded-lg bg-black/60 px-3 py-1.5 font-mono text-[11px] text-white/80 transition-opacity duration-300 ${
            controlsVisible ? 'opacity-100' : 'pointer-events-none opacity-0'
          }`}
        >
          <div className="flex items-center gap-4">
            <span className="flex items-center gap-1.5 text-emerald-400">
              <Activity className="h-3.5 w-3.5" />
              <span>
                {t('live.tracks', 'TRACKS')}: {displayTelemetry.activeTracks}
              </span>
            </span>
            <span className="flex items-center gap-1 text-cyan-300">
              <User className="h-3.5 w-3.5" />
              <span>{displayTelemetry.personCount}</span>
            </span>
            <span className="flex items-center gap-1 text-amber-300">
              <Car className="h-3.5 w-3.5" />
              <span>{displayTelemetry.carCount}</span>
            </span>
          </div>

          <div className="flex items-center gap-3">
            <span className="text-[10px] text-white/50">{t('live.motionHeat', 'MOTION HEAT')}</span>
            <div className="h-1.5 w-16 overflow-hidden rounded-full bg-white/20">
              <div
                className="h-full bg-gradient-to-r from-emerald-400 to-cyan-400 transition-all duration-300"
                style={{ width: `${Math.min(100, displayTelemetry.motionScore * 100)}%` }}
              />
            </div>
          </div>
        </div>
      )}
    </div>
  )
})
