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
        dotClass: 'bg-status-success',
        statusColor: 'text-status-success',
      }
    case 'degraded':
      return {
        text: t('status.degraded', { defaultValue: '网络波动' }),
        dotClass: 'bg-status-warning',
        statusColor: 'text-status-warning',
      }
    case 'offline':
      return {
        text: t('status.offline', { defaultValue: '离线/故障' }),
        dotClass: 'bg-[var(--status-danger)]',
        statusColor: 'text-[var(--status-danger)]',
      }
    case 'unprobed':
    default:
      return {
        text: t('status.unprobed', { defaultValue: '待探测' }),
        dotClass: 'bg-[var(--text-muted)]',
        statusColor: 'text-[var(--text-muted)]',
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
      whileHover={reducedMotion ? undefined : { y: -2 }}
      whileTap={reducedMotion ? undefined : { scale: 0.985 }}
      transition={{
        duration: motionTokens.duration.fast,
        ease: motionTokens.easing.smooth,
      }}
      className={`group relative cursor-pointer overflow-hidden rounded-xl border bg-[var(--bg-secondary)] text-left transition-colors duration-300 hover:shadow-lg ${
        isAlarming
          ? 'border-[var(--status-danger)] shadow-[var(--status-danger)]/40 shadow-lg ring-2 ring-[var(--status-danger)]'
          : isFocused
            ? 'border-status-info shadow-status-info/20 ring-status-info ring-1'
            : 'hover:border-status-info/50 hover:shadow-status-info/10 border-[var(--border)]'
      }`}
    >
      <button
        type="button"
        onClick={() => onSelectHero(camera.cameraId)}
        aria-label={t('live.focusCamera', { name: camera.name })}
        aria-pressed={isFocused}
        className="absolute inset-0 z-10 cursor-pointer rounded-xl focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none focus-visible:ring-inset"
      >
        <span className="sr-only">{t('live.focusCamera', { name: camera.name })}</span>
      </button>

      {/* 告警中微型指示标签 */}
      {isAlarming && (
        <div className="absolute top-2 left-2 z-20 flex items-center gap-1 rounded-md bg-[var(--status-danger-solid)] px-1.5 py-0.5 text-[10px] font-bold text-white shadow-md">
          <ShieldAlert className="h-3 w-3" />
          <span>{t('live.alarmDetected')}</span>
        </div>
      )}

      {/* 微缩播放器视口 */}
      <div className="relative aspect-video w-full">
        {/* 悬停快捷操作组 */}
        <div className="on-dark-surface absolute top-2 right-2 z-20 flex items-center gap-1 rounded-lg bg-black/70 p-1 opacity-100 transition-opacity sm:opacity-0 sm:group-focus-within:opacity-100 sm:group-hover:opacity-100">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onEditCamera(camera)
            }}
            className="min-h-11 min-w-11 rounded p-1 text-white/80 transition-colors hover:bg-white/20 hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0 sm:min-w-0"
            aria-label={t('manage.editCamera')}
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
            className="min-h-11 min-w-11 rounded p-1 text-[var(--status-danger)] transition-colors hover:bg-[var(--status-danger-solid)] hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:min-h-0 sm:min-w-0"
            aria-label={t('manage.deleteCamera')}
            title={t('manage.deleteCamera')}
          >
            <Trash2 className="h-3 w-3" />
          </button>
        </div>

        {isFocused ? (
          <div className="flex h-full w-full flex-col items-center justify-center bg-black/80 p-2 text-center">
            <div className="border-status-info/30 bg-status-info/10 text-status-info flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs">
              <Eye className="h-3 w-3" />
              <span>{t('live.focusedOnHero')}</span>
            </div>
            <span className="mt-1.5 text-[10px] text-[var(--text-muted)]">
              {t('live.auxStreamSleeping')}
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
                ? 'text-[var(--status-danger)]'
                : isFocused
                  ? 'text-status-info'
                  : 'group-hover:text-status-info text-[var(--text-primary)]'
            }`}
          >
            {camera.name}
          </span>
          <span
            className={`rounded px-1.5 py-0.5 font-mono text-[10px] ${
              normalizeProbeStatus(camera.lastProbeStatus) === 'healthy'
                ? 'bg-status-success/10 text-status-success'
                : 'bg-[var(--status-danger-soft)] text-[var(--status-danger)]'
            }`}
          >
            {getResolutionBadge(camera)}
          </span>
        </div>

        <div className="mt-2 flex items-center justify-between text-[11px] text-[var(--text-muted)]">
          {/* 真实目标遥测指标 (对齐 telemetryStore，兼容亮色/暗色高对比度) */}
          <div className="flex items-center gap-2 font-mono">
            <span
              className="text-status-info flex items-center gap-0.5"
              title={
                telemetry ? t('live.targetCount', { count: telemetry.personCount }) : undefined
              }
            >
              <User className="h-3 w-3" />
              <span>{telemetry ? telemetry.personCount : '--'}</span>
            </span>
            <span className="text-status-warning flex items-center gap-0.5">
              <Car className="h-3 w-3" />
              <span>{telemetry ? telemetry.carCount : '--'}</span>
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
