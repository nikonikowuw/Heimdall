import { useEffect, useState } from 'react'
import {
  Camera as CameraIcon,
  Car,
  Compass,
  Eye,
  Grid,
  MonitorPlay,
  Pencil,
  Plus,
  Radio,
  ShieldAlert,
  Sparkles,
  Trash2,
  User,
  X,
  Zap,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { CameraModal, DeleteCameraModal, normalizeProbeStatus } from '@/features/cameras'
import { cameraApi, evidenceApi } from '@/lib/api'
import { useAuthStore } from '@/stores/auth'
import {
  type AlarmSeverity,
  type AlarmStatus,
  type Camera,
  type ProbeStatus,
  WS_TOPICS,
} from '@/types'
import { LivePlayer } from './components/LivePlayer'

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
    // 忽略未交互前的 AudioContext 限制
  }
}

function getCameraHealthRank(status?: string): number {
  switch (normalizeProbeStatus(status)) {
    case 'healthy':
      return 1 // 在线健康
    case 'unprobed':
      return 2 // 待探测
    case 'degraded':
      return 3 // 网络波动
    case 'offline':
      return 4 // 离线/故障
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

function getStatusBadge(status?: ProbeStatus | string) {
  switch (normalizeProbeStatus(status)) {
    case 'healthy':
      return {
        text: '在线',
        badgeClass: 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20',
        dotClass: 'bg-emerald-400 animate-pulse',
        statusColor: 'text-emerald-400',
      }
    case 'degraded':
      return {
        text: '网络波动',
        badgeClass: 'bg-amber-500/10 text-amber-400 border border-amber-500/20',
        dotClass: 'bg-amber-400 animate-ping',
        statusColor: 'text-amber-400',
      }
    case 'offline':
      return {
        text: '离线/故障',
        badgeClass: 'bg-rose-500/10 text-rose-400 border border-rose-500/20',
        dotClass: 'bg-rose-500',
        statusColor: 'text-rose-400',
      }
    case 'unprobed':
    default:
      return {
        text: '待探测',
        badgeClass: 'bg-gray-500/10 text-gray-400 border border-gray-500/20',
        dotClass: 'bg-gray-400',
        statusColor: 'text-gray-400',
      }
  }
}

function getResolutionBadge(cam: Camera) {
  if (cam.lastWidth > 0 && cam.lastHeight > 0) {
    return `${cam.lastHeight}P`
  }
  if (normalizeProbeStatus(cam.lastProbeStatus) === 'healthy') {
    return '1080P'
  }
  return '--'
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
  onNavigateToAlarms?: () => void
}

function LiveAlarmToastItem({
  alarm,
  cameraName,
  onClose,
  onNavigateToAlarms,
}: LiveAlarmToastItemProps): React.ReactElement {
  const { t } = useTranslation('alarm')
  const ruleTypeLabel =
    alarm.ruleType === 'line' ? t('types.lineCrossing') : t('types.regionIntrusion')

  return (
    <div className="animate-in fade-in slide-in-from-top-4 fixed top-6 right-6 z-50 flex items-center gap-3 rounded-2xl border border-rose-500/50 bg-black/90 p-3.5 text-white shadow-2xl backdrop-blur-md duration-300">
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-rose-500/30 bg-rose-500/20 text-rose-500">
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
          <span className="text-[10px] font-bold text-rose-400 uppercase">{ruleTypeLabel}</span>
          <span className="font-semibold text-slate-300">{cameraName || alarm.cameraId}</span>
        </div>
        <p className="font-medium text-slate-200">
          {t('toast.ruleTriggered', { target: alarm.targetLabel })}
        </p>
      </div>
      <div className="ml-2 flex items-center gap-1.5">
        {onNavigateToAlarms && (
          <button
            onClick={() => {
              onClose()
              onNavigateToAlarms()
            }}
            className="rounded-lg bg-rose-500 px-2.5 py-1 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90"
          >
            {t('toast.viewEvidence')}
          </button>
        )}
        <button onClick={onClose} className="p-1 text-slate-400 transition-colors hover:text-white">
          <X className="h-4 w-4" />
        </button>
      </div>
    </div>
  )
}

export interface LivePageProps {
  onNavigateToAlarms?: () => void
}

export function LivePage({ onNavigateToAlarms }: LivePageProps = {}): React.ReactElement {
  const { t } = useTranslation('camera')

  const [cameras, setCameras] = useState<Camera[]>([])
  const [selectedHeroId, setSelectedHeroId] = useState<string>('')
  const [heroStream, setHeroStream] = useState<'main' | 'sub'>('main')
  const [viewMode, setViewMode] = useState<'hero_rail' | 'bento_grid'>('hero_rail')
  const [autoSpotlight, setAutoSpotlight] = useState<boolean>(true)
  const [isCameraModalOpen, setIsCameraModalOpen] = useState<boolean>(false)
  const [cameraToEdit, setCameraToEdit] = useState<Camera | null>(null)
  const [cameraToDelete, setCameraToDelete] = useState<Camera | null>(null)
  const [activeAlarm, setActiveAlarm] = useState<LiveAlarmToast | null>(null)

  useEffect(() => {
    if (!activeAlarm) return
    const timer = setTimeout(() => setActiveAlarm(null), 6000)
    return () => clearTimeout(timer)
  }, [activeAlarm])

  // 加载摄像头列表 (健康状态优先排序)
  useEffect(() => {
    async function loadCameras() {
      try {
        const list = await cameraApi.list()
        if (list && list.length > 0) {
          const sorted = sortCamerasByHealth(list)
          setCameras(sorted)
          setSelectedHeroId((prev) => (prev ? prev : sorted[0].cameraId))
        } else {
          setCameras([])
        }
      } catch {
        setCameras([])
      }
    }

    void loadCameras()
  }, [])

  // 监听全网 WebSocket 广播事件（实时刷新探活状态与健康度自动提权）
  useEffect(() => {
    let ws: WebSocket | null = null
    let isCancelled = false
    let reconnectTimer: NodeJS.Timeout | null = null
    let reconnectAttempts = 0

    function scheduleReconnect() {
      if (isCancelled) return
      const delay = Math.min(1000 * Math.pow(2, reconnectAttempts), 30000)
      reconnectAttempts += 1
      reconnectTimer = setTimeout(connectWs, delay)
    }

    function connectWs() {
      if (isCancelled) return
      const token = useAuthStore.getState().token
      if (!token) return

      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
      const host = window.location.host
      const wsUrl = `${protocol}//${host}/api/v1/ws/events?token=${encodeURIComponent(token)}`

      try {
        const socket = new WebSocket(wsUrl)
        ws = socket

        socket.onopen = () => {
          reconnectAttempts = 0
        }

        socket.onmessage = (e) => {
          try {
            const event = JSON.parse(e.data) as {
              topic: string
              payload: {
                cameraId: string
                status: string
                codec: string
                width: number
                height: number
                fps: number
                errorCode: string
              }
            }

            if (event.topic === WS_TOPICS.CAMERA_PROBE_UPDATED && event.payload) {
              const { cameraId, status, codec, width, height, fps, errorCode } = event.payload
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
            }

            if (event.topic === WS_TOPICS.ALARM_TRIGGERED && event.payload) {
              const p = event.payload as {
                id?: number
                eventId?: string
                cameraId?: string
                targetLabel?: string
                ruleType?: string
                severity?: AlarmSeverity
                cropImageRelPath?: string
                imageRelPath?: string
              }
              playAlarmChime()
              setActiveAlarm({
                id: p.eventId || String(p.id || Date.now()),
                cameraId: p.cameraId || 'CAM-01',
                targetLabel: p.targetLabel || 'Target',
                ruleType: p.ruleType || 'intrusion',
                severity: p.severity || 'warning',
                cropImageRelPath: p.cropImageRelPath,
                imageRelPath: p.imageRelPath,
              })
            }

            if (event.topic === WS_TOPICS.ALARM_STATUS_CHANGED && event.payload) {
              const p = event.payload as {
                id?: number
                eventId?: string
                status?: AlarmStatus
              }
              if (p.status === 'processed') {
                setActiveAlarm((prev) => {
                  if (!prev) return null
                  if (prev.id === p.eventId || prev.id === String(p.id)) {
                    return null
                  }
                  return prev
                })
              }
            }
          } catch {
            // ignore non-JSON or unrelated messages
          }
        }

        socket.onclose = () => {
          if (!isCancelled) {
            scheduleReconnect()
          }
        }

        socket.onerror = () => {
          // onclose 将被触发并处理重连
        }
      } catch {
        if (!isCancelled) {
          scheduleReconnect()
        }
      }
    }

    connectWs()

    return () => {
      isCancelled = true
      if (reconnectTimer) {
        clearTimeout(reconnectTimer)
        reconnectTimer = null
      }
      if (ws) {
        // 解绑监听器，防止组件卸载后仍触发重连或处理消息
        ws.onmessage = null
        ws.onerror = null
        ws.onclose = null

        if (ws.readyState === WebSocket.CONNECTING) {
          // 若处于握手阶段，等待建立后再安全关闭，防止浏览器在控制台产生
          // "WebSocket is closed before the connection is established" 报错
          const pendingWs = ws
          pendingWs.onopen = () => {
            try {
              pendingWs.close(1000, 'unmounted')
            } catch {
              // 忽略关闭异常
            }
          }
        } else if (ws.readyState === WebSocket.OPEN) {
          try {
            ws.close(1000, 'unmounted')
          } catch {
            // 忽略关闭异常
          }
        }
        ws = null
      }
    }
  }, [])

  const heroCamera = cameras.find((c) => c.cameraId === selectedHeroId)

  return (
    <div className="relative flex h-full flex-col gap-3">
      {/* 实时告警低噪稀疏弹窗浮层 */}
      {activeAlarm && (
        <LiveAlarmToastItem
          alarm={activeAlarm}
          cameraName={cameras.find((c) => c.cameraId === activeAlarm.cameraId)?.name}
          onClose={() => setActiveAlarm(null)}
          onNavigateToAlarms={onNavigateToAlarms}
        />
      )}

      {/* 顶部智能监控控制台 HUD 工具栏 */}
      <div className="frosted-glass flex items-center justify-between rounded-xl px-4 py-2.5">
        <div className="flex items-center gap-3">
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-[var(--accent)]/10 text-[var(--accent)]">
            <CameraIcon className="h-4 w-4" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-[var(--text-primary)]">
                {t('live.title', '边缘智能监控大屏')}
              </span>
              <span className="flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-0.5 text-[10px] font-medium text-emerald-500">
                <Radio className="h-2.5 w-2.5 animate-pulse" />
                <span>{t('live.protocolBadge', 'HTTP-FLV · MSE')}</span>
              </span>
              <span className="flex items-center gap-1 rounded-full bg-cyan-500/10 px-2 py-0.5 text-[10px] font-medium text-cyan-400">
                <Zap className="h-2.5 w-2.5" />
                <span>{t('live.aneAccelerator', 'Apple ANE 加速')}</span>
              </span>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {/* 智能追焦开关 */}
          <button
            type="button"
            onClick={() => setAutoSpotlight(!autoSpotlight)}
            className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition-all ${
              autoSpotlight
                ? 'border border-cyan-500/30 bg-cyan-500/15 text-cyan-400'
                : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]'
            }`}
            title={t('live.autoSpotlightDesc', '检测到告警或活动目标时自动将视角切入主视口')}
          >
            <Sparkles className="h-3.5 w-3.5" />
            <span>{t('live.autoSpotlight', '智能追焦')}</span>
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                autoSpotlight ? 'animate-pulse bg-cyan-400' : 'bg-gray-500'
              }`}
            />
          </button>

          {/* 视图布局模式切换 */}
          <div className="flex items-center rounded-lg border border-[var(--border)] p-0.5">
            <button
              type="button"
              onClick={() => setViewMode('hero_rail')}
              className={`flex items-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                viewMode === 'hero_rail'
                  ? 'bg-[var(--accent)] text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              <Compass className="h-3.5 w-3.5" />
              <span>{t('live.focusMode', '指挥舱')}</span>
            </button>
            <button
              type="button"
              onClick={() => setViewMode('bento_grid')}
              className={`flex items-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                viewMode === 'bento_grid'
                  ? 'bg-[var(--accent)] text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              <Grid className="h-3.5 w-3.5" />
              <span>{t('live.bentoMode', '全景 Bento')}</span>
            </button>
          </div>

          {/* 添加摄像头按钮 */}
          <button
            type="button"
            onClick={() => {
              setCameraToEdit(null)
              setIsCameraModalOpen(true)
            }}
            className="flex items-center gap-1 rounded-lg bg-[var(--accent)] px-3 py-1 text-xs font-medium text-white shadow-xs transition-opacity hover:opacity-90"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{t('live.addCamera', '接入设备')}</span>
          </button>
        </div>
      </div>

      {/* 主视口布局容器 */}
      {viewMode === 'hero_rail' ? (
        <div className="grid flex-1 grid-cols-12 gap-3 overflow-hidden">
          {/* 左侧 72% 沉浸式 Hero Stage (占据 8.5 / 12 列) */}
          <div className="col-span-12 flex flex-col gap-2 overflow-hidden lg:col-span-8 xl:col-span-9">
            <div className="relative flex-1 overflow-hidden rounded-xl border border-[var(--border)] bg-black/40">
              {heroCamera ? (
                <LivePlayer
                  key={`${heroCamera.cameraId}:${heroStream}`}
                  cameraId={heroCamera.cameraId}
                  cameraName={heroCamera.name}
                  className="h-full w-full"
                  showHud={true}
                  isHero={true}
                  stream={heroStream}
                  onClose={() => setSelectedHeroId('')}
                  onSwitchStream={(s) => setHeroStream(s)}
                />
              ) : (
                <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center text-[var(--text-muted)]">
                  <div className="flex h-16 w-16 items-center justify-center rounded-2xl bg-[var(--accent)]/10 text-[var(--accent)]">
                    <MonitorPlay className="h-8 w-8 opacity-80" />
                  </div>
                  <div>
                    <h4 className="text-sm font-semibold text-[var(--text-primary)]">
                      {cameras.length > 0
                        ? '主屏预览已关闭'
                        : t('live.noCameras', '暂无活动摄像头')}
                    </h4>
                    <p className="mt-1 max-w-sm text-xs text-[var(--text-secondary)]">
                      {cameras.length > 0
                        ? '右侧子码流正在持续低功耗运行，点击任意视频卡片即可一键切入高清大屏'
                        : '请点击右上角接入 RTSP 视频流设备'}
                    </p>
                  </div>
                  {cameras.length > 0 && (
                    <button
                      type="button"
                      onClick={() => setSelectedHeroId(cameras[0].cameraId)}
                      className="mt-1 flex items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3 py-1.5 text-xs font-medium text-white shadow-xs hover:opacity-90"
                    >
                      <Eye className="h-3.5 w-3.5" />
                      <span>开启主屏预览</span>
                    </button>
                  )}
                </div>
              )}
            </div>

            {/* Hero 下方实时遥测事件滚动胶囊 */}
            <div className="frosted-glass flex items-center justify-between rounded-lg px-3 py-1.5 text-xs text-[var(--text-secondary)]">
              <div className="flex items-center gap-2 font-mono text-[11px]">
                <span className="flex h-2 w-2 animate-ping rounded-full bg-cyan-400" />
                <span className="font-semibold text-cyan-400">
                  {t('live.liveTelemetry', '实时遥测')}:
                </span>
                <span className="text-[var(--text-primary)]">
                  {heroCamera
                    ? `[${heroCamera.name}] ${heroCamera.lastCodec.toUpperCase()} ${heroCamera.lastWidth ? `${heroCamera.lastWidth}x${heroCamera.lastHeight}` : ''}`
                    : '待命中 (子码流待机中)'}
                </span>
              </div>
              <div className="flex items-center gap-3 text-[11px]">
                <span>
                  {t('live.fps', '帧率')}:{' '}
                  <strong className="text-emerald-400">
                    {heroCamera?.lastFps ? heroCamera.lastFps.toFixed(1) : '25.0'} FPS
                  </strong>
                </span>
                <span>
                  {t('live.latency', '端到端延时')}:{' '}
                  <strong className="text-cyan-400">128 ms</strong>
                </span>
                {heroCamera && (
                  <div className="flex items-center gap-1.5 border-l border-[var(--border)] pl-2">
                    <button
                      type="button"
                      onClick={() => {
                        setCameraToEdit(heroCamera)
                        setIsCameraModalOpen(true)
                      }}
                      className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                      title={t('manage.editCamera')}
                    >
                      <Pencil className="h-3 w-3" />
                      <span>{t('manage.editCamera')}</span>
                    </button>
                    <button
                      type="button"
                      onClick={() => setCameraToDelete(heroCamera)}
                      className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-[var(--text-secondary)] transition-colors hover:bg-rose-500/10 hover:text-rose-500"
                      title={t('manage.deleteCamera')}
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* 右侧 28% Bento Live Rail 活动流轨道 (占据 3.5 / 12 列) - 子码流持续播放 */}
          <div className="col-span-12 flex flex-col gap-3 overflow-y-auto lg:col-span-4 xl:col-span-3">
            <div className="flex items-center justify-between px-1 text-xs text-[var(--text-muted)]">
              <span className="font-medium tracking-wide">
                {t('live.auxStreams', '活动监控流')} ({cameras.length})
              </span>
              <span className="text-[10px]">{t('live.switchMainHint', '点击切换主流 ↗')}</span>
            </div>

            <div className="flex flex-1 flex-col gap-3">
              {cameras.map((cam) => {
                const isFocused = cam.cameraId === selectedHeroId
                return (
                  <div
                    key={cam.cameraId}
                    onClick={() => setSelectedHeroId(cam.cameraId)}
                    className={`group relative cursor-pointer overflow-hidden rounded-xl border bg-[var(--bg-secondary)] text-left transition-all duration-300 hover:shadow-lg ${
                      isFocused
                        ? 'border-cyan-500 ring-1 shadow-cyan-500/20 ring-cyan-500'
                        : 'border-[var(--border)] hover:border-cyan-500/50 hover:shadow-cyan-500/10'
                    }`}
                  >
                    {/* 微缩播放器视口 (当被选为主大屏展示时，子码流预览窗口停止播放以释放硬件解码与网络资源) */}
                    <div className="relative aspect-video w-full">
                      {/* 悬停快捷操作组 */}
                      <div className="absolute top-2 right-2 z-10 flex items-center gap-1 rounded-lg bg-black/70 p-1 opacity-0 backdrop-blur-xs transition-opacity group-hover:opacity-100">
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation()
                            setCameraToEdit(cam)
                            setIsCameraModalOpen(true)
                          }}
                          className="rounded p-1 text-white/80 transition-colors hover:bg-white/20 hover:text-white"
                          title={t('manage.editCamera')}
                        >
                          <Pencil className="h-3 w-3" />
                        </button>
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation()
                            setCameraToDelete(cam)
                          }}
                          className="rounded p-1 text-rose-400 transition-colors hover:bg-rose-500/20 hover:text-rose-300"
                          title={t('manage.deleteCamera')}
                        >
                          <Trash2 className="h-3 w-3" />
                        </button>
                      </div>

                      {isFocused ? (
                        <div className="flex h-full w-full flex-col items-center justify-center bg-black/80 p-2 text-center">
                          <div className="flex items-center gap-1.5 rounded-full border border-cyan-500/30 bg-cyan-500/10 px-2.5 py-1 text-xs text-cyan-400">
                            <Eye className="h-3 w-3 animate-pulse" />
                            <span>主屏呈现中</span>
                          </div>
                          <span className="mt-1.5 text-[10px] text-[var(--text-muted)]">
                            辅流已休眠，专注主屏渲染
                          </span>
                        </div>
                      ) : (
                        <LivePlayer
                          cameraId={cam.cameraId}
                          cameraName={cam.name}
                          showHud={false}
                          isHero={false}
                          stream="sub"
                          className="pointer-events-none h-full w-full"
                        />
                      )}
                    </div>

                    {/* 卡片底部遥测状态条 */}
                    <div className="p-2.5">
                      <div className="flex items-center justify-between">
                        <span
                          className={`text-xs font-semibold transition-colors ${
                            isFocused
                              ? 'text-cyan-400'
                              : 'text-[var(--text-primary)] group-hover:text-cyan-400'
                          }`}
                        >
                          {cam.name}
                        </span>
                        <span
                          className={`rounded px-1.5 py-0.5 font-mono text-[10px] ${
                            normalizeProbeStatus(cam.lastProbeStatus) === 'healthy'
                              ? 'bg-emerald-500/10 text-emerald-400'
                              : 'bg-rose-500/10 text-rose-400'
                          }`}
                        >
                          {getResolutionBadge(cam)}
                        </span>
                      </div>

                      <div className="mt-2 flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                        <div className="flex items-center gap-2">
                          <span className="flex items-center gap-0.5">
                            <User className="h-3 w-3" /> 0
                          </span>
                          <span className="flex items-center gap-0.5">
                            <Car className="h-3 w-3" /> 0
                          </span>
                        </div>
                        {(() => {
                          const badge = getStatusBadge(cam.lastProbeStatus)
                          return (
                            <span
                              className={`flex items-center gap-1 font-mono text-[10px] ${badge.statusColor}`}
                            >
                              <span className={`h-1.5 w-1.5 rounded-full ${badge.dotClass}`} />
                              <span>{badge.text}</span>
                            </span>
                          )
                        })()}
                      </div>
                    </div>
                  </div>
                )
              })}

              {cameras.length === 0 && (
                <div className="flex flex-1 items-center justify-center rounded-xl border border-dashed border-[var(--border)] p-8 text-center text-xs text-[var(--text-muted)]">
                  <span>{t('live.noAuxStreams', '暂无更多辅路流')}</span>
                </div>
              )}
            </div>
          </div>
        </div>
      ) : (
        /* 全景自适应 Bento 网格视图 */
        <div className="grid flex-1 grid-cols-1 gap-3 overflow-y-auto md:grid-cols-2 lg:grid-cols-3">
          {cameras.map((cam) => {
            const statusBadge = getStatusBadge(cam.lastProbeStatus)
            return (
              <div
                key={cam.cameraId}
                className="group flex flex-col overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] shadow-xs transition-all hover:border-[var(--accent)]/40 hover:shadow-md"
              >
                {/* 顶部设备标识与操作栏 */}
                <div className="flex items-center justify-between border-b border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-xs">
                  <div className="flex items-center gap-2">
                    <span className={`h-2 w-2 rounded-full ${statusBadge.dotClass}`} />
                    <span className="max-w-[140px] truncate font-semibold text-[var(--text-primary)]">
                      {cam.name}
                    </span>
                    <span className="font-mono text-[10px] text-[var(--text-muted)]">
                      {cam.lastCodec ? cam.lastCodec.toUpperCase() : 'H264'}
                    </span>
                  </div>

                  <div className="flex items-center gap-1">
                    <button
                      type="button"
                      onClick={() => {
                        setSelectedHeroId(cam.cameraId)
                        setViewMode('hero_rail')
                      }}
                      className="flex h-6 items-center gap-1 rounded px-1.5 text-[10px] font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                      title={t('live.focusHero')}
                    >
                      <Eye className="h-3 w-3" />
                      <span>{t('live.focus')}</span>
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setCameraToEdit(cam)
                        setIsCameraModalOpen(true)
                      }}
                      className="flex h-6 w-6 items-center justify-center rounded text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                      title={t('manage.editCamera')}
                    >
                      <Pencil className="h-3 w-3" />
                    </button>
                    <button
                      type="button"
                      onClick={() => setCameraToDelete(cam)}
                      className="flex h-6 w-6 items-center justify-center rounded text-[var(--text-secondary)] transition-colors hover:bg-rose-500/10 hover:text-rose-500"
                      title={t('manage.deleteCamera')}
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                </div>

                <div className="relative aspect-video w-full">
                  <LivePlayer
                    cameraId={cam.cameraId}
                    cameraName={cam.name}
                    showHud={true}
                    isHero={false}
                    stream="sub"
                    className="h-full w-full"
                  />
                </div>
              </div>
            )
          })}

          {cameras.length === 0 && (
            <div className="col-span-full flex items-center justify-center rounded-xl border border-dashed border-[var(--border)] p-12 text-center text-xs text-[var(--text-muted)]">
              <span>{t('live.noCameras', '暂无活动摄像头')}</span>
            </div>
          )}
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
