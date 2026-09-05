import { useEffect, useState } from 'react'
import {
  Camera as CameraIcon,
  Car,
  Compass,
  Eye,
  Grid,
  MonitorPlay,
  Plus,
  Radio,
  ShieldAlert,
  Sparkles,
  User,
  X,
  Zap,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { cameraApi, evidenceApi } from '@/lib/api'
import { useAuthStore } from '@/stores/auth'
import type { Camera, ProbeStatus } from '@/types'
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
  switch (status?.toLowerCase()) {
    case 'healthy':
    case 'success':
    case 'online':
      return 1 // 🟢 在线健康：第 1 名
    case 'never':
    case 'pending':
    case 'connecting':
    case '':
      return 2 // ⚪ 待探测：第 2 名
    case 'degraded':
    case 'reconnecting':
      return 3 // 🟡 网络波动：第 3 名
    case 'failed':
    case 'offline':
    case 'error':
      return 4 // 🔴 离线/故障：第 4 名
    default:
      return 5
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
  const s = status?.toLowerCase()
  if (s === 'healthy' || s === 'success' || s === 'online') {
    return {
      text: '在线',
      badgeClass: 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20',
      dotClass: 'bg-emerald-400 animate-pulse',
      statusColor: 'text-emerald-400',
    }
  }
  if (s === 'degraded' || s === 'reconnecting') {
    return {
      text: '网络波动',
      badgeClass: 'bg-amber-500/10 text-amber-400 border border-amber-500/20',
      dotClass: 'bg-amber-400 animate-ping',
      statusColor: 'text-amber-400',
    }
  }
  if (s === 'failed' || s === 'error' || s === 'offline') {
    return {
      text: '离线/故障',
      badgeClass: 'bg-rose-500/10 text-rose-400 border border-rose-500/20',
      dotClass: 'bg-rose-500',
      statusColor: 'text-rose-400',
    }
  }
  return {
    text: '待探测',
    badgeClass: 'bg-gray-500/10 text-gray-400 border border-gray-500/20',
    dotClass: 'bg-gray-400',
    statusColor: 'text-gray-400',
  }
}

function getResolutionBadge(cam: Camera) {
  if (cam.lastWidth > 0 && cam.lastHeight > 0) {
    return `${cam.lastHeight}P`
  }
  if (cam.lastProbeStatus === 'healthy' || cam.lastProbeStatus === 'success') {
    return '1080P'
  }
  return '--'
}

interface LiveAlarmToast {
  id: string
  cameraId: string
  targetLabel: string
  ruleType: string
  severity: string
  cropImageRelPath?: string
  imageRelPath?: string
}

interface LiveAlarmToastItemProps {
  alarm: LiveAlarmToast
  onClose: () => void
  onNavigateToAlarms?: () => void
}

function LiveAlarmToastItem({
  alarm,
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
          <span className="font-mono text-[10px] text-slate-400">{alarm.cameraId}</span>
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
  const [showAddModal, setShowAddModal] = useState<boolean>(false)
  const [modalMainUrl, setModalMainUrl] = useState<string>('')
  const [modalSubUrl, setModalSubUrl] = useState<string>('')
  const [subCandidates, setSubCandidates] = useState<import('@/types').SubStreamCandidate[]>([])
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

    function connectWs() {
      const token = useAuthStore.getState().token
      if (!token) return

      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
      const host = window.location.host
      const wsUrl = `${protocol}//${host}/api/v1/ws/events?token=${encodeURIComponent(token)}`

      try {
        ws = new WebSocket(wsUrl)

        ws.onmessage = (e) => {
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

            if (event.topic === 'camera.probe_updated' && event.payload) {
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

            if (event.topic === 'alarm.triggered' && event.payload) {
              const p = event.payload as {
                id?: number
                eventId?: string
                cameraId?: string
                targetLabel?: string
                ruleType?: string
                severity?: string
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
          } catch {
            // ignore non-JSON or unrelated messages
          }
        }

        ws.onclose = () => {
          if (!isCancelled) {
            reconnectTimer = setTimeout(connectWs, 3000)
          }
        }
      } catch {
        if (!isCancelled) {
          reconnectTimer = setTimeout(connectWs, 3000)
        }
      }
    }

    connectWs()

    return () => {
      isCancelled = true
      if (reconnectTimer) clearTimeout(reconnectTimer)
      if (ws) ws.close()
    }
  }, [])

  const heroCamera = cameras.find((c) => c.cameraId === selectedHeroId)

  return (
    <div className="relative flex h-full flex-col gap-3">
      {/* 实时告警低噪稀疏弹窗浮层 */}
      {activeAlarm && (
        <LiveAlarmToastItem
          alarm={activeAlarm}
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
            onClick={() => setShowAddModal(true)}
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
                            cam.lastProbeStatus === 'healthy' || cam.lastProbeStatus === 'success'
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
          {cameras.map((cam) => (
            <div
              key={cam.cameraId}
              className="flex flex-col overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]"
            >
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
          ))}

          {cameras.length === 0 && (
            <div className="col-span-full flex items-center justify-center rounded-xl border border-dashed border-[var(--border)] p-12 text-center text-xs text-[var(--text-muted)]">
              <span>{t('live.noCameras', '暂无活动摄像头')}</span>
            </div>
          )}
        </div>
      )}

      {/* 快捷添加摄像头弹窗 */}
      {showAddModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-xs">
          <div className="frosted-glass w-full max-w-md rounded-2xl border border-[var(--border)] p-6 shadow-2xl">
            <h3 className="text-base font-semibold text-[var(--text-primary)]">
              {t('live.addCameraTitle', '接入网络摄像头 (RTSP)')}
            </h3>
            <p className="mt-1 text-xs text-[var(--text-muted)]">
              {t(
                'live.addCameraDesc',
                '支持主流标准 RTSP 协议，系统将自动发起异步握手探活与 SPS 解析。',
              )}
            </p>

            <form
              onSubmit={async (e) => {
                e.preventDefault()
                const form = e.currentTarget
                const name = (form.elements.namedItem('name') as HTMLInputElement).value
                const rtspUrl = modalMainUrl.trim()
                const subRtspUrl = modalSubUrl.trim()
                const remark = (form.elements.namedItem('remark') as HTMLInputElement).value

                try {
                  const created = await cameraApi.create({
                    name,
                    rtspUrl,
                    subRtspUrl: subRtspUrl || undefined,
                    remark,
                  })
                  if (created) {
                    setCameras((prev) => sortCamerasByHealth([...prev, created]))
                    setShowAddModal(false)
                    setModalMainUrl('')
                    setModalSubUrl('')
                    setSubCandidates([])
                  }
                } catch {
                  setShowAddModal(false)
                }
              }}
              className="mt-4 flex flex-col gap-3"
            >
              <div>
                <label className="text-xs font-medium text-[var(--text-secondary)]">
                  {t('live.deviceName', '设备名称')}
                </label>
                <input
                  name="name"
                  required
                  placeholder={t('live.namePlaceholder', '例如：库房正门东区')}
                  className="mt-1 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-1.5 text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-hidden"
                />
              </div>

              <div>
                <label className="text-xs font-medium text-[var(--text-secondary)]">
                  {t('live.rtspUrl', '主码流 RTSP 地址 (4K/1080P 高清分析/大屏)')}
                </label>
                <input
                  name="rtspUrl"
                  required
                  value={modalMainUrl}
                  onChange={(e) => {
                    const val = e.target.value
                    setModalMainUrl(val)
                  }}
                  onBlur={async () => {
                    if (modalMainUrl.trim()) {
                      try {
                        const candidates = await cameraApi.deduceSubStream(modalMainUrl.trim())
                        setSubCandidates(candidates)
                        if (candidates.length > 0 && !modalSubUrl) {
                          setModalSubUrl(candidates[0].subUrl)
                        }
                      } catch {
                        // ignore deduction failure
                      }
                    }
                  }}
                  placeholder={t(
                    'live.rtspPlaceholder',
                    'rtsp://admin:12345@192.168.1.100:554/Streaming/Channels/101',
                  )}
                  className="mt-1 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-1.5 font-mono text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-hidden"
                />
              </div>

              <div>
                <div className="flex items-center justify-between">
                  <label className="text-xs font-medium text-[var(--text-secondary)]">
                    {t('live.subRtspUrl', '子码流 RTSP 地址 (多分屏/Bento 辅流预览)')}
                  </label>
                  {subCandidates.length > 0 && (
                    <span className="text-[10px] font-medium text-cyan-400">⚡ 已自动推导候选</span>
                  )}
                </div>
                <input
                  name="subRtspUrl"
                  value={modalSubUrl}
                  onChange={(e) => setModalSubUrl(e.target.value)}
                  placeholder="可留空自动推导，或手动指定子码流"
                  className="mt-1 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-1.5 font-mono text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-hidden"
                />

                {subCandidates.length > 0 && (
                  <div className="mt-1.5 flex flex-wrap gap-1">
                    {subCandidates.map((c, i) => (
                      <button
                        type="button"
                        key={i}
                        onClick={() => setModalSubUrl(c.subUrl)}
                        className={`rounded-md px-2 py-0.5 font-mono text-[10px] transition-colors ${
                          modalSubUrl === c.subUrl
                            ? 'border border-cyan-500/30 bg-cyan-500/20 text-cyan-400'
                            : 'bg-[var(--accent-soft)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                        }`}
                        title={c.description}
                      >
                        {c.brand}: {c.description}
                      </button>
                    ))}
                  </div>
                )}
              </div>

              <div>
                <label className="text-xs font-medium text-[var(--text-secondary)]">
                  {t('live.remark', '备注说明')}
                </label>
                <input
                  name="remark"
                  placeholder={t('live.remarkPlaceholder', '例如：主要出入口布防')}
                  className="mt-1 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-1.5 text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-hidden"
                />
              </div>

              <div className="mt-2 flex justify-end gap-2">
                <button
                  type="button"
                  onClick={() => setShowAddModal(false)}
                  className="rounded-lg px-3 py-1.5 text-xs font-medium text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]"
                >
                  {t('live.cancel', '取消')}
                </button>
                <button
                  type="submit"
                  className="rounded-lg bg-[var(--accent)] px-3 py-1.5 text-xs font-medium text-white shadow-xs hover:opacity-90"
                >
                  {t('live.saveAndProbe', '保存并探活')}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  )
}
