import React from 'react'
import { Eye, Pencil, ShieldAlert, Trash2 } from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { normalizeProbeStatus } from '@/features/cameras'
import { motionTokens } from '@/lib/motionTokens'
import type { Camera, ProbeStatus } from '@/types'
import { LivePlayer } from '@/components/LivePlayer'

export interface BentoCameraCardProps {
  camera: Camera
  isAlarming?: boolean
  onFocusHero: (cameraId: string) => void
  onEditCamera: (camera: Camera) => void
  onDeleteCamera: (camera: Camera) => void
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
      }
    case 'degraded':
      return {
        text: t('status.degraded', { defaultValue: '网络波动' }),
        dotClass: 'bg-amber-400',
      }
    case 'offline':
      return {
        text: t('status.offline', { defaultValue: '离线/故障' }),
        dotClass: 'bg-[var(--status-danger)]',
      }
    case 'unprobed':
    default:
      return {
        text: t('status.unprobed', { defaultValue: '待探测' }),
        dotClass: 'bg-gray-400',
      }
  }
}

export const BentoCameraCard = React.memo(function BentoCameraCard({
  camera,
  isAlarming = false,
  onFocusHero,
  onEditCamera,
  onDeleteCamera,
}: BentoCameraCardProps): React.ReactElement {
  const { t } = useTranslation('camera')
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
      className={`group flex flex-col overflow-hidden rounded-xl border bg-[var(--bg-secondary)] shadow-xs transition-colors duration-300 ${
        isAlarming
          ? 'border-[var(--status-danger)] shadow-[var(--status-danger)]/40 shadow-lg ring-2 ring-[var(--status-danger)]'
          : 'border-[var(--border)] hover:border-[var(--accent)]/40 hover:shadow-md'
      }`}
    >
      {/* 顶部设备标识与操作栏 */}
      <div className="flex items-center justify-between border-b border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-xs">
        <div className="flex items-center gap-2">
          <span className={`h-2 w-2 rounded-full ${statusBadge.dotClass}`} />
          <span className="max-w-[140px] truncate font-semibold text-[var(--text-primary)]">
            {camera.name}
          </span>
          <span className="font-mono text-[10px] text-[var(--text-muted)]">
            {camera.lastCodec ? camera.lastCodec.toUpperCase() : 'H264'}
          </span>
          {isAlarming && (
            <span className="py-0.2 flex items-center gap-0.5 rounded bg-[var(--status-danger-soft)] px-1.5 font-mono text-[9px] font-bold text-[var(--status-danger)]">
              <ShieldAlert className="h-2.5 w-2.5" />
              <span>ALARM</span>
            </span>
          )}
        </div>

        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={() => onFocusHero(camera.cameraId)}
            className="flex min-h-11 items-center gap-1 rounded px-2 text-[10px] font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:h-6 sm:min-h-0"
            aria-label={t('live.focusHero')}
            title={t('live.focusHero')}
          >
            <Eye className="h-3 w-3" />
            <span>{t('live.focus')}</span>
          </button>
          <button
            type="button"
            onClick={() => onEditCamera(camera)}
            className="min-h-11 min-w-11 rounded text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:h-6 sm:min-h-0 sm:min-w-0"
            aria-label={t('manage.editCamera')}
            title={t('manage.editCamera')}
          >
            <Pencil className="h-3 w-3" />
          </button>
          <button
            type="button"
            onClick={() => onDeleteCamera(camera)}
            className="min-h-11 min-w-11 rounded text-[var(--text-secondary)] transition-colors hover:bg-[var(--status-danger-soft)] hover:text-[var(--status-danger)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none sm:h-6 sm:min-h-0 sm:min-w-0"
            aria-label={t('manage.deleteCamera')}
            title={t('manage.deleteCamera')}
          >
            <Trash2 className="h-3 w-3" />
          </button>
        </div>
      </div>

      <div className="relative aspect-video w-full">
        <LivePlayer
          cameraId={camera.cameraId}
          cameraName={camera.name}
          showHud={true}
          isHero={false}
          stream="sub"
          videoCodec={camera.lastCodec}
          className="h-full w-full"
        />
      </div>
    </motion.div>
  )
})
