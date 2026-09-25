import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Camera as CameraIcon,
  ChevronLeft,
  ChevronRight,
  Compass,
  CornerUpLeft,
  Eye,
  Grid,
  LoaderCircle,
  MonitorPlay,
  Pencil,
  Plus,
  Radio,
  RefreshCw,
  Search,
  ShieldAlert,
  Sparkles,
  Trash2,
  X,
  Zap,
} from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { CameraModal, DeleteCameraModal, normalizeProbeStatus } from '@/features/cameras'
import { cameraApi, evidenceApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import { systemApi } from '@/lib/system-api'
import { wsClient } from '@/lib/wsClient'
import {
  type AlarmSeverity,
  type AlarmStatus,
  type Camera,
  type ProbeStatus,
  WS_TOPICS,
} from '@/types'
import { AuxCameraCard } from './components/AuxCameraCard'
import { BentoCameraCard } from './components/BentoCameraCard'
import { LivePlayer } from '@/components/LivePlayer'

function playAlarmChime() {
  try {
    const AudioCtx =
      window.AudioContext ||
      (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext
    if (!AudioCtx) return
    const ctx = new AudioCtx()
    const osc = ctx.createOscillator()
    const gain = ctx.createGain()
    osc.type = 'sine'
    osc.frequency.setValueAtTime(880, ctx.currentTime)
    osc.frequency.exponentialRampToValueAtTime(440, ctx.currentTime + 0.3)
    gain.gain.setValueAtTime(0.15, ctx.currentTime)
    gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.3)
    osc.connect(gain)
    gain.connect(ctx.destination)
    osc.start()
    osc.stop(ctx.currentTime + 0.3)
  } catch {
    // 忽略未交互前的 AudioContext 自动播放限制
  }
}

function getCameraHealthRank(status?: string): number {
  switch (normalizeProbeStatus(status)) {
    case 'healthy':
      return 1
    case 'unprobed':
      return 2
    case 'degraded':
      return 3
    case 'offline':
      return 4
  }
}

function sortCamerasByHealth(list: Camera[]): Camera[] {
  return [...list].sort((a, b) => {
    const rankA = getCameraHealthRank(a.lastProbeStatus)
    const rankB = getCameraHealthRank(b.lastProbeStatus)
    if (rankA !== rankB) return rankA - rankB
    if (a.lastSuccessAt && b.lastSuccessAt && a.lastSuccessAt !== b.lastSuccessAt) {
      return b.lastSuccessAt - a.lastSuccessAt
    }
    return b.id - a.id
  })
}

interface LiveAlarmToast {
  id: string
  cameraId: string
  targetLabel: string
  ruleType: string
  severity: AlarmSeverity
  cropImageRelPath?: string
  imageRelPath?: string
}

interface LiveAlarmToastItemProps {
  alarm: LiveAlarmToast
  cameraName?: string
  onClose: () => void
  onFocusCamera?: (cameraId: string) => void
  onNavigateToAlarms?: () => void
  reducedMotion?: boolean | null
}

function LiveAlarmToastItem({
  alarm,
  cameraName,
  onClose,
  onFocusCamera,
  onNavigateToAlarms,
  reducedMotion,
}: LiveAlarmToastItemProps): React.ReactElement {
  const { t } = useTranslation(['alarm', 'camera'])
  const ruleTypeLabel =
    alarm.ruleType === 'line' ? t('alarm:types.lineCrossing') : t('alarm:types.regionIntrusion')

  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: reducedMotion ? 0 : -20, scale: reducedMotion ? 1 : 0.95 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: reducedMotion ? 0 : -14, scale: reducedMotion ? 1 : 0.95 }}
      transition={{
        duration: reducedMotion ? motionTokens.duration.fast : motionTokens.duration.normal,
        ease: motionTokens.easing.smooth,
      }}
      className="on-dark-surface fixed top-6 right-6 z-50 flex items-center gap-3 rounded-2xl border border-[var(--status-danger)]/50 bg-black/90 p-3.5 text-white shadow-2xl backdrop-blur-md"
    >
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-[var(--status-danger-border)] bg-[var(--status-danger)]/20 text-[var(--status-danger)]">
        <ShieldAlert className="h-5 w-5" />
      </div>
      {alarm.cropImageRelPath && (
        <div className="h-10 w-10 shrink-0 overflow-hidden rounded-lg border border-white/20 bg-black">
          <img
            src={evidenceApi.getImageUrl(alarm.cropImageRelPath)}
            alt="Thumb"
            className="h-full w-full object-cover"
          />
        </div>
      )}
      <div className="space-y-0.5 text-xs">
        <div className="flex items-center gap-2">
          <span className="text-[10px] font-bold text-[var(--status-danger)] uppercase">
            {ruleTypeLabel}
          </span>
          <span className="font-semibold text-slate-300">{cameraName || alarm.cameraId}</span>
        </div>
        <p className="font-medium text-slate-200">
          {t('alarm:toast.ruleTriggered', { target: alarm.targetLabel })}
        </p>
      </div>
      <div className="ml-2 flex items-center gap-1.5">
        {onFocusCamera && (
          <button
            type="button"
            onClick={() => {
              onFocusCamera(alarm.cameraId)
            }}
            className="rounded-lg bg-cyan-500/20 px-2.5 py-1 text-xs font-semibold text-cyan-300 transition-all hover:bg-cyan-500/30"
          >
            {t('camera:live.focus')}
          </button>
        )}
        {onNavigateToAlarms && (
          <button
            type="button"
            onClick={() => {
              onClose()
              onNavigateToAlarms()
            }}
            className="rounded-lg bg-[var(--status-danger-solid)] px-2.5 py-1 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90"
          >
            {t('alarm:toast.viewEvidence')}
          </button>
        )}
        <button
          type="button"
          onClick={onClose}
          className="p-1 text-slate-400 transition-colors hover:text-white"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    </motion.div>
  )
}

export type BentoGridSplit = 1 | 4 | 9 | 'all'

const ALL_CAMERAS_PAGE_SIZE = 50

type CameraListLoadState = 'loading' | 'error'

interface CameraListStatusProps {
  state: CameraListLoadState
  onRetry: () => void
}

function CameraListStatus({ state, onRetry }: CameraListStatusProps): React.ReactElement {
  const { t } = useTranslation('camera')

  if (state === 'loading') {
    return (
      <div
        className="flex h-full min-h-48 flex-col items-center justify-center gap-2 text-sm text-[var(--text-secondary)]"
        role="status"
        aria-live="polite"
      >
        <LoaderCircle className="h-5 w-5 animate-spin text-[var(--accent)]" aria-hidden="true" />
        <span>{t('live.loadingCameras')}</span>
      </div>
    )
  }

  return (
    <div
      className="flex h-full min-h-48 flex-col items-center justify-center gap-3 text-center"
      role="alert"
    >
      <span className="text-sm text-[var(--text-secondary)]">{t('live.cameraListLoadFailed')}</span>
      <button
        type="button"
        onClick={onRetry}
        className="flex min-h-11 items-center gap-2 rounded-md border border-[var(--border)] px-3 text-xs font-medium text-[var(--text-primary)] transition-colors hover:bg-[var(--accent-soft)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
      >
        <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
        <span>{t('live.retryLoad')}</span>
      </button>
    </div>
  )
}

export interface LivePageProps {
  onNavigateToAlarms?: () => void
}

export function LivePage({ onNavigateToAlarms }: LivePageProps = {}): React.ReactElement {
  const { t } = useTranslation('camera')
  const reducedMotion = useReducedMotion()

  const [cameras, setCameras] = useState<Camera[]>([])
  const camerasRef = useRef(cameras)
  camerasRef.current = cameras
  const [cameraLoadState, setCameraLoadState] = useState<'loading' | 'ready' | 'error'>('loading')
  const [cameraLoadAttempt, setCameraLoadAttempt] = useState(0)
  const [selectedHeroId, setSelectedHeroId] = useState<string>('')
  const [heroStream, setHeroStream] = useState<'main' | 'sub'>('main')
  const [heroAudioEnabled, setHeroAudioEnabled] = useState<boolean>(false)
  const [viewMode, setViewMode] = useState<'hero_rail' | 'bento_grid'>('hero_rail')
  const [autoSpotlight, setAutoSpotlight] = useState<boolean>(true)
  const [spotlightBanner, setSpotlightBanner] = useState<{
    cameraName: string
    revertId: string | null
  } | null>(null)
  const [recentAlarms, setRecentAlarms] = useState<Record<string, number>>({})

  // Bento 网格分屏与分页
  const [gridSplit, setGridSplit] = useState<BentoGridSplit>(4)
  const [gridPage, setGridPage] = useState<number>(1)

  // 辅流轨道搜索与过滤
  const [auxSearch, setAuxSearch] = useState<string>('')
  const [auxFilter, setAuxFilter] = useState<'all' | 'healthy' | 'alarm'>('all')

  // 硬件与系统信息
  const [hwLabel, setHwLabel] = useState<string>('')
  const [heroLatency, setHeroLatency] = useState<{
    value: number
    kind: 'renderLag' | 'bufferLag'
  } | null>(null)

  const handleHeroLatencyChange = useCallback((value: number, kind: 'renderLag' | 'bufferLag') => {
    setHeroLatency({ value, kind })
  }, [])

  const handleRetryCameraLoad = useCallback(() => {
    setCameraLoadAttempt((attempt) => attempt + 1)
  }, [])

  const handleToggleHeroAudio = useCallback(() => {
    setHeroAudioEnabled((prev) => !prev)
  }, [])

  const handleCloseHero = useCallback(() => {
    setSelectedHeroId('')
  }, [])

  const handleSwitchHeroStream = useCallback((s: 'main' | 'sub') => {
    setHeroLatency(null)
    setHeroStream(s)
  }, [])

  // 模态框
  const [isCameraModalOpen, setIsCameraModalOpen] = useState<boolean>(false)
  const [cameraToEdit, setCameraToEdit] = useState<Camera | null>(null)
  const [cameraToDelete, setCameraToDelete] = useState<Camera | null>(null)
  const [activeAlarm, setActiveAlarm] = useState<LiveAlarmToast | null>(null)

  // 告警 Toast 自动超时关闭
  useEffect(() => {
    if (!activeAlarm) return
    const timer = setTimeout(() => setActiveAlarm(null), 6000)
    return () => clearTimeout(timer)
  }, [activeAlarm])

  // 追焦提示条自动超时关闭
  useEffect(() => {
    if (!spotlightBanner) return
    const timer = setTimeout(() => setSpotlightBanner(null), 4500)
    return () => clearTimeout(timer)
  }, [spotlightBanner])

  // 定期清理过期的近期告警状态 (8 秒后移除高亮发光)
  useEffect(() => {
    const interval = setInterval(() => {
      const now = Date.now()
      setRecentAlarms((prev) => {
        let changed = false
        const next = { ...prev }
        for (const [id, exp] of Object.entries(next)) {
          if (exp <= now) {
            delete next[id]
            changed = true
          }
        }
        return changed ? next : prev
      })
    }, 2000)
    return () => clearInterval(interval)
  }, [])

  useEffect(() => {
    setHeroLatency(null)
  }, [selectedHeroId, heroStream])

  useEffect(() => {
    let cancelled = false

    async function loadInitialData() {
      setCameraLoadState('loading')
      try {
        const list = await cameraApi.list()
        if (cancelled) return
        if (list && list.length > 0) {
          const sorted = sortCamerasByHealth(list)
          setCameras(sorted)
          setSelectedHeroId((prev) =>
            sorted.some((camera) => camera.cameraId === prev) ? prev : sorted[0].cameraId,
          )
        } else {
          setCameras([])
          setSelectedHeroId('')
        }
        setCameraLoadState('ready')
      } catch {
        if (!cancelled) {
          setCameras([])
          setCameraLoadState('error')
        }
      }

      try {
        const overview = await systemApi.getOverview()
        if (cancelled) return
        if (overview) {
          if (overview.deviceModel && overview.deviceModel.length > 0) {
            setHwLabel(overview.deviceModel)
          } else if (overview.npu) {
            setHwLabel('NPU 加速')
          }
        }
      } catch {
        // 忽略非关键概览加载异常
      }
    }

    void loadInitialData()
    return () => {
      cancelled = true
    }
  }, [cameraLoadAttempt])

  // 监听全网 WebSocket 广播事件（探活更新、告警触发、告警状态变更）
  useEffect(() => {
    const unProbe = wsClient.subscribe<{
      cameraId: string
      status: string
      codec: string
      width: number
      height: number
      fps: number
      errorCode: string
    }>(WS_TOPICS.CAMERA_PROBE_UPDATED, (payload) => {
      if (!payload) return
      const { cameraId, status, codec, width, height, fps, errorCode } = payload
      setCameras((prev) => {
        const next = prev.map((cam) => {
          if (cam.cameraId === cameraId) {
            return {
              ...cam,
              lastProbeStatus: status as ProbeStatus,
              lastCodec: codec || cam.lastCodec,
              lastWidth: width || cam.lastWidth,
              lastHeight: height || cam.lastHeight,
              lastFps: fps !== undefined ? fps : cam.lastFps,
              lastProbeErrorCode: errorCode,
            }
          }
          return cam
        })
        return sortCamerasByHealth(next)
      })
    })

    const unAlarm = wsClient.subscribe<{
      id?: number
      eventId?: string
      cameraId?: string
      targetLabel?: string
      ruleType?: string
      severity?: AlarmSeverity
      cropImageRelPath?: string
      imageRelPath?: string
    }>(WS_TOPICS.ALARM_TRIGGERED, (p) => {
      if (!p) return
      playAlarmChime()

      const targetCamId = p.cameraId || 'CAM-01'

      setActiveAlarm({
        id: p.eventId || String(p.id || Date.now()),
        cameraId: targetCamId,
        targetLabel: p.targetLabel || 'Target',
        ruleType: p.ruleType || 'intrusion',
        severity: p.severity || 'warning',
        cropImageRelPath: p.cropImageRelPath,
        imageRelPath: p.imageRelPath,
      })

      // 记录近期告警机位 (激活呼吸发光框持续 8 秒)
      setRecentAlarms((prev) => ({
        ...prev,
        [targetCamId]: Date.now() + 8000,
      }))

      // 智能追焦真正闭环联动
      if (autoSpotlight && targetCamId) {
        setSelectedHeroId((currentHero) => {
          if (currentHero !== targetCamId) {
            const targetCam = camerasRef.current.find((c) => c.cameraId === targetCamId)
            setSpotlightBanner({
              cameraName: targetCam?.name || targetCamId,
              revertId: currentHero || null,
            })
            return targetCamId
          }
          return currentHero
        })
      }
    })

    const unAlarmStatus = wsClient.subscribe<{
      id?: number
      eventId?: string
      status?: AlarmStatus
    }>(WS_TOPICS.ALARM_STATUS_CHANGED, (p) => {
      if (p?.status === 'processed') {
        setActiveAlarm((prev) => {
          if (!prev) return null
          if (prev.id === p.eventId || prev.id === String(p.id)) {
            return null
          }
          return prev
        })
      }
    })

    return () => {
      unProbe()
      unAlarm()
      unAlarmStatus()
    }
  }, [autoSpotlight])

  // 过滤后的辅流摄像头列表
  const filteredAuxCameras = useMemo(() => {
    let list = cameras
    if (auxSearch.trim()) {
      const q = auxSearch.trim().toLowerCase()
      list = list.filter(
        (c) => c.name.toLowerCase().includes(q) || c.cameraId.toLowerCase().includes(q),
      )
    }
    if (auxFilter === 'healthy') {
      list = list.filter((c) => normalizeProbeStatus(c.lastProbeStatus) === 'healthy')
    } else if (auxFilter === 'alarm') {
      const now = Date.now()
      list = list.filter((c) => (recentAlarms[c.cameraId] ?? 0) > now)
    }
    return list
  }, [cameras, auxSearch, auxFilter, recentAlarms])

  // Bento 网格分页切片；全景模式每页最多挂载 50 路播放器。
  const bentoPageSize = gridSplit === 'all' ? ALL_CAMERAS_PAGE_SIZE : gridSplit
  const totalBentoPages = Math.max(1, Math.ceil(cameras.length / bentoPageSize))
  const currentBentoPage = Math.min(gridPage, totalBentoPages)
  const paginatedBentoCameras = useMemo(() => {
    const start = (currentBentoPage - 1) * bentoPageSize
    return cameras.slice(start, start + bentoPageSize)
  }, [cameras, currentBentoPage, bentoPageSize])

  const heroCamera = cameras.find((c) => c.cameraId === selectedHeroId)
  let auxiliaryEmptyMessageKey = 'live.noAuxStreams'
  if (cameraLoadState === 'loading') {
    auxiliaryEmptyMessageKey = 'live.loadingCameras'
  } else if (cameraLoadState === 'error') {
    auxiliaryEmptyMessageKey = 'live.cameraListLoadFailed'
  } else if (auxSearch || auxFilter !== 'all') {
    auxiliaryEmptyMessageKey = 'live.noFilteredCameras'
  }

  let heroLatencyLabelKey = 'live.latency'
  if (heroLatency?.kind === 'renderLag') heroLatencyLabelKey = 'live.renderLag'
  if (heroLatency?.kind === 'bufferLag') heroLatencyLabelKey = 'live.bufferLag'

  // 切换追焦机位
  const handleSelectHero = useCallback((cameraId: string) => {
    setHeroLatency(null)
    setSelectedHeroId(cameraId)
    setSpotlightBanner(null)
  }, [])

  const handleEditCamera = useCallback((camera: Camera) => {
    setCameraToEdit(camera)
    setIsCameraModalOpen(true)
  }, [])

  const handleDeleteCamera = useCallback((camera: Camera) => {
    setCameraToDelete(camera)
  }, [])

  const handleFocusHeroFromGrid = useCallback(
    (cameraId: string) => {
      handleSelectHero(cameraId)
      setViewMode('hero_rail')
    },
    [handleSelectHero],
  )

  return (
    <div className="relative flex h-full flex-col gap-3">
      {/* 实时告警低噪稀疏弹窗浮层 (AnimatePresence) */}
      <AnimatePresence mode="popLayout">
        {activeAlarm && (
          <LiveAlarmToastItem
            key={activeAlarm.id}
            alarm={activeAlarm}
            cameraName={cameras.find((c) => c.cameraId === activeAlarm.cameraId)?.name}
            onClose={() => setActiveAlarm(null)}
            onFocusCamera={(camId) => handleSelectHero(camId)}
            onNavigateToAlarms={onNavigateToAlarms}
            reducedMotion={reducedMotion}
          />
        )}
      </AnimatePresence>

      {/* 智能追焦机位切换提示横幅 (AnimatePresence) */}
      <AnimatePresence mode="wait">
        {spotlightBanner && (
          <motion.div
            key={spotlightBanner.cameraName}
            initial={{ opacity: 0, y: reducedMotion ? 0 : -16, scale: reducedMotion ? 1 : 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: reducedMotion ? 0 : -12, scale: reducedMotion ? 1 : 0.96 }}
            transition={{
              duration: reducedMotion ? motionTokens.duration.fast : motionTokens.duration.normal,
              ease: motionTokens.easing.smooth,
            }}
            className="absolute top-14 left-1/2 z-30 flex -translate-x-1/2 items-center gap-3 rounded-full border border-cyan-500/40 bg-black/85 px-4 py-1.5 text-xs text-white shadow-2xl backdrop-blur-md"
          >
            <div className="flex items-center gap-1.5 font-medium text-cyan-300">
              <Sparkles className="h-3.5 w-3.5 text-cyan-400" />
              <span>{t('live.spotlightSwitchHint', { name: spotlightBanner.cameraName })}</span>
            </div>
            {spotlightBanner.revertId && (
              <button
                type="button"
                onClick={() => {
                  if (spotlightBanner.revertId) {
                    setSelectedHeroId(spotlightBanner.revertId)
                    setSpotlightBanner(null)
                  }
                }}
                className="flex items-center gap-1 rounded bg-white/10 px-2 py-0.5 text-[11px] font-medium text-slate-200 transition-colors hover:bg-white/20 hover:text-white"
              >
                <CornerUpLeft className="h-3 w-3" />
                <span>{t('live.revertSpotlight')}</span>
              </button>
            )}
            <button
              type="button"
              onClick={() => setSpotlightBanner(null)}
              className="text-slate-400 hover:text-white"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </motion.div>
        )}
      </AnimatePresence>

      {/* 顶部智能监控控制台 HUD 工具栏 */}
      <div className="frosted-glass flex items-center justify-between rounded-xl px-4 py-2.5">
        <div className="flex items-center gap-3">
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-[var(--accent)]/10 text-[var(--accent)]">
            <CameraIcon className="h-4 w-4" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-[var(--text-primary)]">
                {t('live.title')}
              </span>
              <span className="flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                <Radio className="h-2.5 w-2.5" />
                <span>{t('live.webcodecsBadge')}</span>
              </span>
              <span className="flex items-center gap-1 rounded-full bg-cyan-500/10 px-2 py-0.5 text-[10px] font-medium text-cyan-700 dark:text-cyan-400">
                <Zap className="h-2.5 w-2.5" />
                <span>{hwLabel || t('live.hwAcceleratorReady')}</span>
              </span>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {/* 智能追焦开关 */}
          <motion.button
            type="button"
            whileTap={{ scale: 0.96 }}
            onClick={() => setAutoSpotlight((enabled) => !enabled)}
            aria-pressed={autoSpotlight}
            className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition-all focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
              autoSpotlight
                ? 'border border-cyan-500/30 bg-cyan-500/10 text-cyan-700 dark:bg-cyan-500/15 dark:text-cyan-400'
                : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]'
            }`}
            title={t('live.autoSpotlightDesc')}
          >
            <Sparkles className="h-3.5 w-3.5" />
            <span>{t('live.autoSpotlight')}</span>
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                autoSpotlight ? 'bg-cyan-400' : 'bg-gray-500'
              }`}
            />
          </motion.button>

          {/* 视图布局模式切换 (Motion layoutId Pill) */}
          <div className="flex items-center rounded-lg border border-[var(--border)] p-0.5">
            <button
              type="button"
              onClick={() => setViewMode('hero_rail')}
              aria-pressed={viewMode === 'hero_rail'}
              className="relative flex items-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium transition-colors focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
            >
              {viewMode === 'hero_rail' && (
                <motion.div
                  layoutId="live-view-mode-pill"
                  className="absolute inset-0 rounded-md bg-[var(--accent)] shadow-xs"
                  transition={{
                    type: 'spring',
                    stiffness: 480,
                    damping: 36,
                  }}
                />
              )}
              <span
                className={`relative z-10 flex items-center gap-1 ${
                  viewMode === 'hero_rail'
                    ? 'text-white'
                    : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                }`}
              >
                <Compass className="h-3.5 w-3.5" />
                <span>{t('live.focusMode')}</span>
              </span>
            </button>
            <button
              type="button"
              onClick={() => setViewMode('bento_grid')}
              aria-pressed={viewMode === 'bento_grid'}
              className="relative flex items-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium transition-colors focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
            >
              {viewMode === 'bento_grid' && (
                <motion.div
                  layoutId="live-view-mode-pill"
                  className="absolute inset-0 rounded-md bg-[var(--accent)] shadow-xs"
                  transition={{
                    type: 'spring',
                    stiffness: 480,
                    damping: 36,
                  }}
                />
              )}
              <span
                className={`relative z-10 flex items-center gap-1 ${
                  viewMode === 'bento_grid'
                    ? 'text-white'
                    : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                }`}
              >
                <Grid className="h-3.5 w-3.5" />
                <span>{t('live.bentoMode')}</span>
              </span>
            </button>
          </div>

          {/* 添加摄像头按钮 */}
          <motion.button
            type="button"
            whileTap={{ scale: 0.96 }}
            whileHover={{ scale: 1.02 }}
            onClick={() => {
              setCameraToEdit(null)
              setIsCameraModalOpen(true)
            }}
            className="flex min-h-11 items-center gap-1 rounded-lg bg-[var(--accent)] px-3 py-1 text-xs font-medium text-white shadow-xs transition-opacity hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{t('live.addCamera')}</span>
          </motion.button>
        </div>
      </div>

      {/* 主视口布局容器 */}
      {viewMode === 'hero_rail' ? (
        <div className="grid flex-1 grid-cols-12 gap-3 overflow-hidden">
          {/* 左侧沉浸式 Hero Stage (占据 8.5 / 12 列) */}
          <div className="col-span-12 flex flex-col gap-2 overflow-hidden lg:col-span-8 xl:col-span-9">
            <div className="relative flex-1 overflow-hidden rounded-xl border border-[var(--border)] bg-black/40">
              {cameraLoadState !== 'ready' ? (
                <CameraListStatus state={cameraLoadState} onRetry={handleRetryCameraLoad} />
              ) : heroCamera ? (
                <LivePlayer
                  key={`${heroCamera.cameraId}:${heroStream}`}
                  cameraId={heroCamera.cameraId}
                  cameraName={heroCamera.name}
                  className="h-full w-full"
                  showHud={true}
                  isHero={true}
                  stream={heroStream}
                  videoCodec={heroCamera.lastCodec}
                  audioEnabled={heroAudioEnabled}
                  onToggleAudio={handleToggleHeroAudio}
                  onLatencyChange={handleHeroLatencyChange}
                  onClose={handleCloseHero}
                  onSwitchStream={handleSwitchHeroStream}
                />
              ) : (
                <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center text-[var(--text-muted)]">
                  <div className="flex h-16 w-16 items-center justify-center rounded-2xl bg-[var(--accent)]/10 text-[var(--accent)]">
                    <MonitorPlay className="h-8 w-8 opacity-80" />
                  </div>
                  <div>
                    <h4 className="text-sm font-semibold text-[var(--text-primary)]">
                      {cameras.length > 0 ? t('live.previewClosed') : t('live.noCameras')}
                    </h4>
                    <p className="mt-1 max-w-sm text-xs text-[var(--text-secondary)]">
                      {cameras.length > 0 ? t('live.previewClosedDesc') : t('live.emptyCameraDesc')}
                    </p>
                  </div>
                  {cameras.length > 0 && (
                    <button
                      type="button"
                      onClick={() => handleSelectHero(cameras[0].cameraId)}
                      className="mt-1 flex min-h-11 items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3 py-1.5 text-xs font-medium text-white shadow-xs hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                    >
                      <Eye className="h-3.5 w-3.5" />
                      <span>{t('live.startHeroPreview')}</span>
                    </button>
                  )}
                </div>
              )}
            </div>

            {/* Hero 下方实时遥测事件滚动胶囊 */}
            <div className="frosted-glass flex items-center justify-between rounded-lg px-3 py-1.5 text-xs text-[var(--text-secondary)]">
              <div className="flex items-center gap-2 font-mono text-[11px]">
                <span className="flex h-2 w-2 rounded-full bg-cyan-500 dark:bg-cyan-400" />
                <span className="font-semibold text-cyan-700 dark:text-cyan-400">
                  {t('live.liveTelemetry')}:
                </span>
                <span className="text-[var(--text-primary)]">
                  {heroCamera
                    ? `[${heroCamera.name}] ${heroCamera.lastCodec ? heroCamera.lastCodec.toUpperCase() : 'H264'} ${heroCamera.lastWidth ? `${heroCamera.lastWidth}x${heroCamera.lastHeight}` : ''}`
                    : t('live.savingMode')}
                </span>
              </div>
              <div className="flex items-center gap-3 text-[11px]">
                <span>
                  {t('live.fps')}:{' '}
                  <strong className="text-emerald-600 dark:text-emerald-400">
                    {heroCamera?.lastFps ? heroCamera.lastFps.toFixed(1) : '--'} FPS
                  </strong>
                </span>
                <span>
                  {t(heroLatencyLabelKey)}:{' '}
                  <strong className="text-cyan-700 dark:text-cyan-400">
                    {heroLatency ? `${heroLatency.value} ms` : '--'}
                  </strong>
                </span>
                {heroCamera && (
                  <div className="flex items-center gap-1.5 border-l border-[var(--border)] pl-2">
                    <button
                      type="button"
                      onClick={() => handleEditCamera(heroCamera)}
                      className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                      title={t('manage.editCamera')}
                    >
                      <Pencil className="h-3 w-3" />
                      <span>{t('manage.editCamera')}</span>
                    </button>
                    <button
                      type="button"
                      onClick={() => handleDeleteCamera(heroCamera)}
                      className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-[var(--text-secondary)] transition-colors hover:bg-[var(--status-danger-soft)] hover:text-[var(--status-danger)]"
                      title={t('manage.deleteCamera')}
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* 右侧 Bento Live Rail 活动流轨道 (占据 3.5 / 12 列) */}
          <div className="col-span-12 flex flex-col gap-2.5 overflow-hidden lg:col-span-4 xl:col-span-3">
            {/* 辅流轨道头部状态与快速搜索 */}
            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between px-1 text-xs text-[var(--text-muted)]">
                <span className="font-medium tracking-wide">
                  {t('live.auxStreams')} ({cameras.length})
                </span>
                <span className="text-[10px]">{t('live.switchMainHint')}</span>
              </div>

              {/* 搜索与过滤工具栏 */}
              <div className="flex items-center gap-1.5">
                <div className="relative flex-1">
                  <Search className="absolute top-1/2 left-2 h-3 w-3 -translate-y-1/2 text-slate-400" />
                  <input
                    type="text"
                    value={auxSearch}
                    onChange={(e) => setAuxSearch(e.target.value)}
                    aria-label={t('live.searchPlaceholder')}
                    placeholder={t('live.searchPlaceholder')}
                    className="h-7 w-full rounded-md border border-[var(--border)] bg-[var(--bg-secondary)] pr-2 pl-7 text-[11px] text-[var(--text-primary)] outline-none placeholder:text-slate-400 focus:border-cyan-500 focus-visible:ring-2 focus-visible:ring-[var(--ring)]"
                  />
                  {auxSearch && (
                    <button
                      type="button"
                      onClick={() => setAuxSearch('')}
                      className="absolute top-1/2 right-1.5 flex h-8 w-8 -translate-y-1/2 items-center justify-center rounded text-slate-400 hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                      aria-label={t('live.clearSearch')}
                      title={t('live.clearSearch')}
                    >
                      <X className="h-3 w-3" />
                    </button>
                  )}
                </div>

                <div
                  className="flex items-center rounded-md border border-[var(--border)] p-0.5 text-[10px]"
                  role="group"
                  aria-label={t('live.filterCameras')}
                >
                  {(['all', 'healthy', 'alarm'] as const).map((filter) => (
                    <button
                      key={filter}
                      type="button"
                      onClick={() => setAuxFilter(filter)}
                      aria-pressed={auxFilter === filter}
                      className="relative min-h-11 rounded px-1.5 py-0.5 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0"
                    >
                      {auxFilter === filter && (
                        <motion.div
                          layoutId="aux-filter-pill"
                          className={`absolute inset-0 rounded ${
                            filter === 'all'
                              ? 'bg-[var(--accent)] shadow-xs'
                              : filter === 'healthy'
                                ? 'bg-emerald-500/20'
                                : 'bg-[var(--status-danger)]/20'
                          }`}
                          transition={{
                            type: 'spring',
                            stiffness: 480,
                            damping: 36,
                          }}
                        />
                      )}
                      <span
                        className={`relative z-10 ${
                          auxFilter === filter
                            ? filter === 'all'
                              ? 'font-medium text-white'
                              : filter === 'healthy'
                                ? 'font-medium text-emerald-700 dark:text-emerald-400'
                                : 'font-medium text-[var(--status-danger)]'
                            : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                        }`}
                      >
                        {filter === 'all'
                          ? t('live.filterAll')
                          : filter === 'healthy'
                            ? t('live.filterHealthy')
                            : t('live.filterAlarm')}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            </div>

            {/* 辅路流卡片纵向滚动列表 */}
            <div className="flex flex-1 flex-col gap-3 overflow-y-auto pr-0.5">
              {filteredAuxCameras.map((cam) => {
                const isFocused = cam.cameraId === selectedHeroId
                const isAlarming = Boolean(
                  recentAlarms[cam.cameraId] && recentAlarms[cam.cameraId] > Date.now(),
                )

                return (
                  <AuxCameraCard
                    key={cam.cameraId}
                    camera={cam}
                    isFocused={isFocused}
                    isAlarming={isAlarming}
                    onSelectHero={handleSelectHero}
                    onEditCamera={handleEditCamera}
                    onDeleteCamera={handleDeleteCamera}
                  />
                )
              })}

              {filteredAuxCameras.length === 0 && (
                <div className="flex flex-1 items-center justify-center rounded-xl border border-dashed border-[var(--border)] p-8 text-center text-xs text-[var(--text-muted)]">
                  <span>{t(auxiliaryEmptyMessageKey)}</span>
                </div>
              )}
            </div>
          </div>
        </div>
      ) : (
        /* 全景 Bento 网格视图 (支持 1 / 4 / 9 宫格与分页) */
        <div className="flex flex-1 flex-col gap-2 overflow-hidden">
          {/* Bento 工具栏：分屏选择与分页控制 */}
          <div className="flex flex-col items-start gap-2 px-1 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex flex-wrap items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] p-0.5 text-xs">
              {([1, 4, 9, 'all'] as const).map((split) => (
                <button
                  key={split}
                  type="button"
                  onClick={() => {
                    setGridSplit(split)
                    setGridPage(1)
                  }}
                  aria-pressed={gridSplit === split}
                  className="relative rounded px-2.5 py-1 text-[11px] font-medium transition-colors focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                >
                  {gridSplit === split && (
                    <motion.div
                      layoutId="bento-split-pill"
                      className="absolute inset-0 rounded bg-[var(--accent)] shadow-xs"
                      transition={{
                        type: 'spring',
                        stiffness: 480,
                        damping: 36,
                      }}
                    />
                  )}
                  <span
                    className={`relative z-10 ${
                      gridSplit === split
                        ? 'text-white'
                        : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                    }`}
                  >
                    {t(`live.grid${split === 'all' ? 'All' : split}`)}
                  </span>
                </button>
              ))}
            </div>

            {/* 分页控制器 */}
            {totalBentoPages > 1 && (
              <div className="flex items-center gap-2 text-xs text-[var(--text-secondary)]">
                <span>
                  {t('live.pageIndicator', {
                    current: currentBentoPage,
                    total: totalBentoPages,
                  })}
                </span>
                <div className="flex items-center gap-1">
                  <button
                    type="button"
                    disabled={currentBentoPage <= 1}
                    onClick={() => setGridPage(Math.max(1, currentBentoPage - 1))}
                    className="flex h-11 w-11 items-center justify-center rounded border border-[var(--border)] text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-30 sm:h-6 sm:w-6"
                    aria-label={t('live.prevPage')}
                    title={t('live.prevPage')}
                  >
                    <ChevronLeft className="h-3.5 w-3.5" />
                  </button>
                  <button
                    type="button"
                    disabled={currentBentoPage >= totalBentoPages}
                    onClick={() => setGridPage(Math.min(totalBentoPages, currentBentoPage + 1))}
                    className="flex h-11 w-11 items-center justify-center rounded border border-[var(--border)] text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-30 sm:h-6 sm:w-6"
                    aria-label={t('live.nextPage')}
                    title={t('live.nextPage')}
                  >
                    <ChevronRight className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            )}
          </div>

          {/* 响应式宫格容器 */}
          <div
            className={`grid flex-1 gap-3 overflow-y-auto ${
              gridSplit === 1
                ? 'grid-cols-1'
                : gridSplit === 4
                  ? 'grid-cols-1 md:grid-cols-2'
                  : gridSplit === 9
                    ? 'grid-cols-1 md:grid-cols-2 lg:grid-cols-3'
                    : 'grid-cols-1 md:grid-cols-2 lg:grid-cols-3'
            }`}
          >
            {paginatedBentoCameras.map((cam) => {
              const isAlarming = Boolean(
                recentAlarms[cam.cameraId] && recentAlarms[cam.cameraId] > Date.now(),
              )

              return (
                <BentoCameraCard
                  key={cam.cameraId}
                  camera={cam}
                  isAlarming={isAlarming}
                  onFocusHero={handleFocusHeroFromGrid}
                  onEditCamera={handleEditCamera}
                  onDeleteCamera={handleDeleteCamera}
                />
              )
            })}

            {cameras.length === 0 && (
              <div className="col-span-full flex items-center justify-center rounded-xl border border-dashed border-[var(--border)] p-12 text-center text-xs text-[var(--text-muted)]">
                {cameraLoadState === 'ready' ? (
                  <span>{t('live.noCameras')}</span>
                ) : (
                  <CameraListStatus state={cameraLoadState} onRetry={handleRetryCameraLoad} />
                )}
              </div>
            )}
          </div>
        </div>
      )}

      {/* 统一添加/编辑摄像头模态框 */}
      <CameraModal
        isOpen={isCameraModalOpen}
        camera={cameraToEdit}
        onClose={() => {
          setIsCameraModalOpen(false)
          setCameraToEdit(null)
        }}
        onSuccess={(saved) => {
          setCameras((prev) => {
            const exists = prev.some((c) => c.cameraId === saved.cameraId)
            const updated = exists
              ? prev.map((c) => (c.cameraId === saved.cameraId ? saved : c))
              : [...prev, saved]
            return sortCamerasByHealth(updated)
          })
          setCameraLoadState('ready')
          if (!selectedHeroId) {
            setSelectedHeroId(saved.cameraId)
          }
        }}
      />

      {/* 摄像头删除确认模态框 */}
      <DeleteCameraModal
        isOpen={Boolean(cameraToDelete)}
        camera={cameraToDelete}
        onClose={() => setCameraToDelete(null)}
        onSuccess={(deletedId) => {
          setCameras((prev) => {
            const updated = prev.filter((c) => c.cameraId !== deletedId)
            if (selectedHeroId === deletedId) {
              setSelectedHeroId(updated.length > 0 ? updated[0].cameraId : '')
            }
            return updated
          })
        }}
      />
    </div>
  )
}
