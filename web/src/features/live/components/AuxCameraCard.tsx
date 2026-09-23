import React from 'react'
import { Car, Eye, Pencil, ShieldAlert, Trash2, User } from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { normalizeProbeStatus } from '@/features/cameras'
import { motionTokens } from '@/lib/motionTokens'
import type { Camera, ProbeStatus } from '@/types'
import { useCameraTelemetry } from '../hooks/useCameraTelemetry'
import { LivePlayer } from '@/components/LivePlayer'

export interface AuxCameraCardProps {
  camera: Camera
  isFocused: boolean
  isAlarming?: boolean
  onSelectHero: (cameraId: string) => void
  onEditCamera: (camera: Camera) => void
  onDeleteCamera: (camera: Camera) => void
}

function getResolutionBadge(cam: Camera): string {
  if (cam.lastWidth > 0 && cam.lastHeight > 0) {
    return `${cam.lastHeight}P`
  }
  if (normalizeProbeStatus(cam.lastProbeStatus) === 'healthy') {
    return '1080P'
  }
  return '--'
}

function getStatusBadge(
  t: (key: string, opts?: { defaultValue?: string }) => string,
  status?: ProbeStatus | string,
) {
  switch (normalizeProbeStatus(status)) {
    case 'healthy':
      return {
        text: t('status.online', { defaultValue: '在线' }),
        dotClass: 'bg-emerald-400',
        statusColor: 'text-emerald-400',
      }
    case 'degraded':
      return {
        text: t('status.degraded', { defaultValue: '网络波动' }),
        dotClass: 'bg-amber-400',
        statusColor: 'text-amber-400',
      }
    case 'offline':
      return {
        text: t('status.offline', { defaultValue: '离线/故障' }),
        dotClass: 'bg-rose-500',
        statusColor: 'text-rose-400',
      }
    case 'unprobed':
    default:
      return {
        text: t('status.unprobed', { defaultValue: '待探测' }),
        dotClass: 'bg-gray-400',
        statusColor: 'text-gray-400',
      }
  }
}

export const AuxCameraCard = React.memo(function AuxCameraCard({
  camera,
  isFocused,
  isAlarming = false,
  onSelectHero,
  onEditCamera,
  onDeleteCamera,
}: AuxCameraCardProps): React.ReactElement {
  const { t } = useTranslation('camera')
  const telemetry = useCameraTelemetry(camera.cameraId)
  const statusBadge = getStatusBadge(t, camera.lastProbeStatus)
  const reducedMotion = useReducedMotion()

  return (
    <motion.div
      onClick={() => onSelectHero(camera.cameraId)}
      whileHover={reducedMotion ? undefined : { y: -2 }}
      whileTap={reducedMotion ? undefined : { scale: 0.985 }}
      transition={{
        duration: motionTokens.duration.fast,
        ease: motionTokens.easing.smooth,
      }}
      className={`group relative cursor-pointer overflow-hidden rounded-xl border bg-[var(--bg-secondary)] text-left transition-colors duration-300 hover:shadow-lg ${
        isAlarming
          ? 'border-rose-500 shadow-lg ring-2 shadow-rose-500/40 ring-rose-500'
          : isFocused
            ? 'border-cyan-500 ring-1 shadow-cyan-500/20 ring-cyan-500'
            : 'border-[var(--border)] hover:border-cyan-500/50 hover:shadow-cyan-500/10'
      }`}
    >
      {/* 告警中微型指示标签 */}
      {isAlarming && (
        <div className="absolute top-2 left-2 z-20 flex items-center gap-1 rounded-md bg-rose-500/90 px-1.5 py-0.5 text-[10px] font-bold text-white shadow-md">
          <ShieldAlert className="h-3 w-3" />
          <span>{t('live.alarmDetected')}</span>
        </div>
      )}

      {/* 微缩播放器视口 */}
      <div className="relative aspect-video w-full">
        {/* 悬停快捷操作组 */}
        <div className="absolute top-2 right-2 z-20 flex items-center gap-1 rounded-lg bg-black/70 p-1 opacity-0 transition-opacity group-hover:opacity-100">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onEditCamera(camera)
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
              onDeleteCamera(camera)
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
              <Eye className="h-3 w-3" />
              <span>主屏呈现中</span>
            </div>
            <span className="mt-1.5 text-[10px] text-[var(--text-muted)]">
              辅流已休眠，专注主屏渲染
            </span>
          </div>
        ) : (
          <LivePlayer
            cameraId={camera.cameraId}
            cameraName={camera.name}
            showHud={false}
            isHero={false}
            stream="sub"
            videoCodec={camera.lastCodec}
            className="pointer-events-none h-full w-full"
          />
        )}
      </div>

      {/* 卡片底部遥测状态条 */}
      <div className="p-2.5">
        <div className="flex items-center justify-between">
          <span
            className={`text-xs font-semibold transition-colors ${
              isAlarming
                ? 'text-rose-500 dark:text-rose-400'
                : isFocused
                  ? 'text-cyan-700 dark:text-cyan-400'
                  : 'text-[var(--text-primary)] group-hover:text-cyan-700 dark:group-hover:text-cyan-400'
            }`}
          >
            {camera.name}
          </span>
          <span
            className={`rounded px-1.5 py-0.5 font-mono text-[10px] ${
              normalizeProbeStatus(camera.lastProbeStatus) === 'healthy'
                ? 'bg-emerald-500/10 text-emerald-400'
                : 'bg-rose-500/10 text-rose-400'
            }`}
          >
            {getResolutionBadge(camera)}
          </span>
        </div>

        <div className="mt-2 flex items-center justify-between text-[11px] text-[var(--text-muted)]">
          {/* 真实目标遥测指标 (对齐 telemetryStore，兼容亮色/暗色高对比度) */}
          <div className="flex items-center gap-2 font-mono">
            <span
              className="flex items-center gap-0.5 text-cyan-700 dark:text-cyan-400"
              title={t('live.targetCount', { count: telemetry?.personCount ?? 0 })}
            >
              <User className="h-3 w-3" />
              <span>{telemetry ? telemetry.personCount : 0}</span>
            </span>
            <span className="flex items-center gap-0.5 text-amber-700 dark:text-amber-400">
              <Car className="h-3 w-3" />
              <span>{telemetry ? telemetry.carCount : 0}</span>
            </span>
          </div>

          <span
            className={`flex items-center gap-1 font-mono text-[10px] ${statusBadge.statusColor}`}
          >
            <span className={`h-1.5 w-1.5 rounded-full ${statusBadge.dotClass}`} />
            <span>{statusBadge.text}</span>
          </span>
        </div>
      </div>
    </motion.div>
  )
})
