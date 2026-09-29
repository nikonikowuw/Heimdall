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
  if (cam.lastWidth >= 3840 || cam.lastHeight >= 2160) {
    return '4K'
  }
  if (cam.lastHeight > 0) {
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
        dotClass: 'bg-[var(--status-success)]',
        statusColor: 'text-[var(--status-success)]',
      }
    case 'degraded':
      return {
        text: t('status.degraded', { defaultValue: '网络波动' }),
        dotClass: 'bg-[var(--status-warning)]',
        statusColor: 'text-[var(--status-warning)]',
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
        dotClass: 'bg-white/40',
        statusColor: 'text-white/80',
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
      className={`group on-dark-surface relative aspect-video w-full cursor-pointer overflow-hidden rounded-xl border bg-[var(--video-surface)] text-left transition-colors duration-300 hover:shadow-lg ${
        isAlarming
          ? 'border-[var(--status-danger)] shadow-[var(--status-danger)]/40 shadow-lg ring-2 ring-[var(--status-danger)]'
          : isFocused
            ? 'border-status-info shadow-status-info/20 ring-status-info ring-1'
            : 'hover:border-status-info/50 hover:shadow-status-info/10 border-[var(--border)]'
      }`}
    >
      {/* 整卡点击切换为主屏焦点 */}
      <button
        type="button"
        onClick={() => onSelectHero(camera.cameraId)}
        aria-label={t('live.focusCamera', { name: camera.name })}
        aria-pressed={isFocused}
        className="absolute inset-0 z-10 cursor-pointer rounded-xl focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none focus-visible:ring-inset"
      >
        <span className="sr-only">{t('live.focusCamera', { name: camera.name })}</span>
      </button>

      {/* 画面视口（主屏休眠占位 vs 实时辅流播放器） */}
      <div className="relative h-full w-full">
        {isFocused ? (
          <div className="flex h-full w-full flex-col items-center justify-center bg-black/85 p-2 text-center select-none">
            <div className="flex items-center gap-1.5 rounded-full border border-[var(--status-info)]/40 bg-[var(--status-info)]/15 px-3 py-1 text-xs font-semibold text-[var(--status-info)] shadow-xs">
              <Eye className="h-3.5 w-3.5" />
              <span>{t('live.focusedOnHero')}</span>
            </div>
            <span className="mt-1.5 text-[10px] font-medium text-white/70">
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

      {/* 顶部悬浮 HUD：机位信息胶囊与快捷操作 */}
      <div className="pointer-events-none absolute inset-x-0 top-0 z-20 flex items-center justify-between p-2 text-xs">
        <div className="flex max-w-[calc(100%-4.25rem)] min-w-0 items-center gap-1.5 rounded-lg border border-white/20 bg-black/80 px-2 py-1 shadow-md backdrop-blur-xs">
          <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${statusBadge.dotClass}`} />
          <span
            className={`truncate font-semibold transition-colors ${
              isAlarming
                ? 'text-[var(--status-danger)]'
                : isFocused
                  ? 'text-[var(--status-info)]'
                  : 'text-white group-hover:text-[var(--status-info)]'
            }`}
            title={camera.name}
          >
            {camera.name}
          </span>
          <span className="py-0.2 shrink-0 rounded border border-white/20 bg-white/10 px-1 font-mono text-[9px] font-semibold text-white/90">
            {getResolutionBadge(camera)}
          </span>
          {isAlarming && (
            <span className="py-0.2 flex shrink-0 items-center gap-0.5 rounded bg-[var(--status-danger-solid)] px-1.5 font-mono text-[9px] font-bold text-white shadow-xs">
              <ShieldAlert className="h-2.5 w-2.5" />
              <span>{t('live.alarmDetected')}</span>
            </span>
          )}
        </div>

        {/* 悬停快捷操作组 */}
        <div className="pointer-events-auto flex items-center gap-1 opacity-100 transition-opacity sm:opacity-0 sm:group-focus-within:opacity-100 sm:group-hover:opacity-100">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onEditCamera(camera)
            }}
            className="flex h-6 w-6 items-center justify-center rounded-lg border border-white/20 bg-black/80 text-white/80 shadow-xs backdrop-blur-xs transition-colors hover:bg-white/25 hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
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
            className="flex h-6 w-6 items-center justify-center rounded-lg border border-white/20 bg-black/80 text-[var(--status-danger)] shadow-xs backdrop-blur-xs transition-colors hover:bg-[var(--status-danger-solid)] hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
            aria-label={t('manage.deleteCamera')}
            title={t('manage.deleteCamera')}
          >
            <Trash2 className="h-3 w-3" />
          </button>
        </div>
      </div>

      {/* 底部悬浮 HUD：实时遥测与网络状态 */}
      <div className="pointer-events-none absolute inset-x-0 bottom-0 z-20 flex items-center justify-between p-2 text-[10px]">
        {/* 真实目标遥测指标 (对齐 telemetryStore) */}
        <div className="pointer-events-auto flex items-center gap-3 rounded-lg border border-white/20 bg-black/80 px-2.5 py-1 font-mono shadow-md backdrop-blur-xs">
          <span
            className="flex items-center gap-1 text-[var(--status-info)]"
            title={telemetry ? t('live.targetCount', { count: telemetry.personCount }) : undefined}
          >
            <User className="h-3.5 w-3.5" />
            <span className="font-semibold text-white/95">
              {telemetry ? telemetry.personCount : '--'}
            </span>
          </span>
          <span
            className="flex items-center gap-1 text-[var(--status-warning)]"
            title={
              telemetry
                ? t('live.vehicleCount', {
                    count: telemetry.carCount,
                    defaultValue: '{{count}} 车辆',
                  })
                : undefined
            }
          >
            <Car className="h-3.5 w-3.5" />
            <span className="font-semibold text-white/95">
              {telemetry ? telemetry.carCount : '--'}
            </span>
          </span>
        </div>

        {/* 设备网络探测状态 */}
        <div className="rounded-lg border border-white/20 bg-black/80 px-2.5 py-1 font-mono shadow-md backdrop-blur-xs">
          <span
            className={`flex items-center gap-1.5 text-[10px] font-semibold ${statusBadge.statusColor}`}
          >
            <span className={`h-1.5 w-1.5 rounded-full ${statusBadge.dotClass}`} />
            <span>{statusBadge.text}</span>
          </span>
        </div>
      </div>
    </motion.div>
  )
})
