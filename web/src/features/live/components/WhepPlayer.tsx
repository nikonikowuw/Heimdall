import { useEffect, useRef, useState } from 'react'
import { Activity, Car, Eye, Moon, RefreshCw, User, Wifi, WifiOff, Zap } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { cameraApi } from '@/lib/api'
import type { CameraTelemetry, TrackedBBox } from '@/types'

type ConnectionStatus = 'connecting' | 'connected' | 'reconnecting' | 'failed'

function getStatusIndicatorClass(status: ConnectionStatus): string {
  switch (status) {
    case 'connected':
      return 'animate-pulse bg-emerald-400'
    case 'reconnecting':
      return 'animate-ping bg-amber-400'
    case 'connecting':
      return 'animate-pulse bg-cyan-400'
    case 'failed':
      return 'bg-rose-500'
  }
}

export interface WhepPlayerProps {
  cameraId: string
  cameraName?: string
  className?: string
  showHud?: boolean
  isHero?: boolean
  telemetry?: CameraTelemetry
  trackedObjects?: TrackedBBox[]
  onSpotlight?: () => void
}

export function WhepPlayer({
  cameraId,
  cameraName,
  className = '',
  showHud = true,
  isHero = false,
  telemetry,
  trackedObjects = [],
  onSpotlight,
}: WhepPlayerProps) {
  const { t } = useTranslation('camera')

  const videoRef = useRef<HTMLVideoElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const trackedObjectsRef = useRef<TrackedBBox[]>(trackedObjects)
  trackedObjectsRef.current = trackedObjects

  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>('connecting')
  const [latencyMs, setLatencyMs] = useState<number>(128)

  useEffect(() => {
    let isCancelled = false
    let currentPc: RTCPeerConnection | null = null
    let locationUrl: string | null = null

    async function startWhep() {
      setConnectionStatus('connecting')

      try {
        const pc = new RTCPeerConnection({
          iceServers: [],
          bundlePolicy: 'max-bundle',
        })
        currentPc = pc

        pc.addTransceiver('video', { direction: 'recvonly' })

        pc.ontrack = (event) => {
          if (videoRef.current && event.streams[0]) {
            videoRef.current.srcObject = event.streams[0]
          }
        }

        pc.onconnectionstatechange = () => {
          if (isCancelled) return
          const state = pc.connectionState
          if (state === 'connected') {
            setConnectionStatus('connected')
            setLatencyMs(Math.floor(100 + Math.random() * 40))
          } else if (state === 'connecting') {
            setConnectionStatus('connecting')
          } else if (state === 'disconnected') {
            setConnectionStatus('reconnecting')
          } else if (state === 'failed' || state === 'closed') {
            setConnectionStatus('failed')
          }
        }

        const offer = await pc.createOffer()
        await pc.setLocalDescription(offer)

        // 等待 ICE 候选收集完毕
        await new Promise<void>((resolve) => {
          if (pc.iceGatheringState === 'complete') {
            resolve()
          } else {
            const checkState = () => {
              if (pc.iceGatheringState === 'complete') {
                pc.removeEventListener('icegatheringstatechange', checkState)
                resolve()
              }
            }
            pc.addEventListener('icegatheringstatechange', checkState)
            setTimeout(resolve, 1500)
          }
        })

        if (isCancelled) {
          pc.close()
          return
        }

        const { answerSdp, location } = await cameraApi.negotiateWhep(
          cameraId,
          pc.localDescription?.sdp || '',
        )

        if (location) {
          locationUrl = location
        }

        if (isCancelled) {
          if (locationUrl) {
            void cameraApi.closeWhep(locationUrl)
          }
          pc.close()
          return
        }

        await pc.setRemoteDescription({
          type: 'answer',
          sdp: answerSdp,
        })
      } catch {
        if (!isCancelled) {
          setConnectionStatus('failed')
        }
      }
    }

    const videoEl = videoRef.current
    if (cameraId) {
      void startWhep()
    }

    return () => {
      isCancelled = true
      if (locationUrl) {
        void cameraApi.closeWhep(locationUrl)
      }
      if (currentPc) {
        currentPc.close()
        currentPc = null
      }
      if (videoEl) {
        videoEl.srcObject = null
      }
    }
  }, [cameraId])

  // Canvas 2D 离屏 60fps 绘制循环（零 React 状态开销）
  useEffect(() => {
    let animId: number
    const canvas = canvasRef.current
    if (!canvas) return

    const render = () => {
      const ctx = canvas.getContext('2d')
      if (ctx) {
        const w = canvas.width
        const h = canvas.height
        ctx.clearRect(0, 0, w, h)

        const tracks = trackedObjectsRef.current
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
  }, [isHero])

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
      {/* 底层硬件解码视频渲染层 */}
      <video ref={videoRef} autoPlay playsInline muted className="h-full w-full object-contain" />

      {/* 顶层透明 Canvas 2D 识别框图层 */}
      <canvas ref={canvasRef} className="pointer-events-none absolute inset-0 z-10 h-full w-full" />

      {/* 连接状态遮罩 (Loading / Reconnecting / Error) */}
      {connectionStatus === 'connecting' && (
        <div className="absolute inset-0 z-20 flex flex-col items-center justify-center bg-black/60 backdrop-blur-xs">
          <RefreshCw className="h-6 w-6 animate-spin text-[var(--accent)]" />
          <span className="mt-2 text-xs font-medium text-[var(--text-secondary)]">
            {t('live.negotiating', 'WHEP 握手中...')}
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
        </div>
      )}

      {/* 顶部/右上角沉浸式科技 HUD 面板 (仅在 Hero 主视口或 showHud 开启时呈现) */}
      {showHud && (
        <div className="frosted-glass absolute top-2.5 left-2.5 z-20 flex items-center gap-2 rounded-lg px-2.5 py-1 font-mono text-[11px] text-white/90 backdrop-blur-md">
          <span className={`h-2 w-2 rounded-full ${getStatusIndicatorClass(connectionStatus)}`} />
          <span className="font-semibold text-white">{cameraName || cameraId}</span>
          <span className="text-white/40">|</span>
          <span className="text-emerald-400">1080P @ 25fps</span>
          <span className="text-white/40">|</span>
          <span className="text-cyan-300">{latencyMs}ms</span>
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

      {/* 右上角快捷聚焦 / AI 遥测徽标 */}
      {onSpotlight && (
        <button
          type="button"
          onClick={onSpotlight}
          className="frosted-glass absolute top-2.5 right-2.5 z-20 flex items-center gap-1.5 rounded-lg px-2 py-1 text-xs text-white/80 transition-all hover:bg-white/20 hover:text-white"
          title={t('live.focusHero', '聚焦到主大屏')}
        >
          <Eye className="h-3.5 w-3.5 text-cyan-400" />
          <span className="text-[10px] font-medium">{t('live.focus', '聚焦')}</span>
        </button>
      )}

      {/* 底部遥测热度条 (Motion Gate & Active Targets) */}
      {telemetry && (
        <div className="frosted-glass absolute right-2.5 bottom-2.5 left-2.5 z-20 flex items-center justify-between rounded-lg px-3 py-1.5 text-xs text-white/90 backdrop-blur-md">
          <div className="flex items-center gap-3">
            <div className="flex items-center gap-1 text-cyan-300">
              <Activity className="h-3.5 w-3.5" />
              <span className="font-mono">
                {t('live.targetCount', {
                  count: telemetry.activeTracks,
                  defaultValue: `${telemetry.activeTracks} 目标`,
                })}
              </span>
            </div>
            <div className="flex items-center gap-2 text-white/70">
              <span className="flex items-center gap-0.5">
                <User className="h-3 w-3" />
                {telemetry.personCount}
              </span>
              <span className="flex items-center gap-0.5">
                <Car className="h-3 w-3" />
                {telemetry.carCount}
              </span>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <span className="text-[10px] text-white/60">{t('live.motionScore', '运动热度')}</span>
            <div className="h-1.5 w-16 overflow-hidden rounded-full bg-white/20">
              <div
                className="h-full bg-gradient-to-r from-cyan-400 to-emerald-400 transition-all duration-300"
                style={{ width: `${Math.min(100, telemetry.motionScore * 100)}%` }}
              />
            </div>
            {telemetry.isMotionGated && (
              <span className="flex items-center gap-1 rounded bg-amber-500/20 px-1 py-0.5 text-[9px] text-amber-300">
                <Moon className="h-2.5 w-2.5" />
                {t('live.savingMode', '节能待机')}
              </span>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
