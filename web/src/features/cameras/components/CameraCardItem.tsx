import React, { useMemo } from 'react'
import {
  Check,
  Clock,
  Copy,
  Cpu,
  FileText,
  Film,
  Maximize2,
  Pencil,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Split,
  Trash2,
} from 'lucide-react'
import { motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import { formatRelativeTime } from '@/lib/time'
import type { Camera, TaskSummaryDto } from '@/types'
import { getProbeBadge, normalizeProbeStatus } from '../cameraStatus'

const STATUS_BORDER_STYLES: Record<string, string> = {
  healthy:
    'border-[var(--border)] hover:border-emerald-500/35 hover:shadow-[0_12px_32px_-6px_rgba(16,185,129,0.14)]',
  degraded:
    'border-[var(--border)] hover:border-amber-500/35 hover:shadow-[0_12px_32px_-6px_rgba(245,158,11,0.14)]',
  offline:
    'border-[var(--border)] hover:border-[var(--destructive)]/30 hover:shadow-[0_12px_32px_-6px_rgba(239,68,68,0.12)]',
  unprobed: 'border-[var(--border)] hover:border-[var(--border-strong)] hover:shadow-lg',
}

const STATUS_GLOW_STYLES: Record<string, string> = {
  healthy: 'bg-gradient-to-r from-emerald-500/0 via-emerald-400 to-emerald-500/0',
  degraded: 'bg-gradient-to-r from-amber-500/0 via-amber-400 to-amber-500/0',
  offline: 'bg-gradient-to-r from-rose-500/0 via-rose-500 to-rose-500/0',
  unprobed: 'bg-transparent',
}

export interface CameraCardItemProps {
  camera: Camera
  boundTask?: TaskSummaryDto
  isProbing: boolean
  probeFeedback?: 'success' | 'failed'
  isCopied: boolean
  onCopyRtsp: (cameraId: string, url: string) => void
  onManualProbe: (camera: Camera) => void
  onEdit: (camera: Camera) => void
  onDelete: (camera: Camera) => void
  onNavigateToTasks?: (camera: Camera) => void
}

export function CameraCardItem({
  camera,
  boundTask,
  isProbing,
  probeFeedback,
  isCopied,
  onCopyRtsp,
  onManualProbe,
  onEdit,
  onDelete,
  onNavigateToTasks,
}: CameraCardItemProps): React.ReactElement {
  const { t, i18n } = useTranslation('camera')
  const { t: tc } = useTranslation('common')
  const probeBadge = getProbeBadge(camera.lastProbeStatus, t)
  const normalizedStatus = normalizeProbeStatus(camera.lastProbeStatus)
  const isGb = camera.protocol === 'gb28181'

  const statusBorderStyle = STATUS_BORDER_STYLES[normalizedStatus] || STATUS_BORDER_STYLES.unprobed
  const statusGlowStyle = STATUS_GLOW_STYLES[normalizedStatus] || STATUS_GLOW_STYLES.unprobed

  // 是否为 H.265 硬件加速
  const isH265 = Boolean(
    camera.lastCodec &&
    (camera.lastCodec.toLowerCase().includes('265') ||
      camera.lastCodec.toLowerCase().includes('hevc')),
  )

  // 分辨率等级智能微标识别
  const resTier = useMemo(() => {
    const w = camera.lastWidth || 0
    const h = camera.lastHeight || 0
    if (w >= 3840 || h >= 2160) return '4K'
    if (w >= 2560 || h >= 1440) return '2K'
    if (w >= 1920 || h >= 1080) return '1080P'
    if (w >= 1280 || h >= 720) return '720P'
    return null
  }, [camera.lastWidth, camera.lastHeight])

  // 探活时间时效性计算（标准 Intl 相对时间转换）
  const probeFreshness = useMemo(() => {
    const timestamp = camera.lastProbeAt || camera.lastSuccessAt
    if (!timestamp) return t('manage.pendingProbeHint', { defaultValue: '待首次探活' })
    return formatRelativeTime(timestamp, i18n.language)
  }, [camera.lastProbeAt, camera.lastSuccessAt, i18n.language, t])

  // 分析码流展示标签
  const streamModeLabel = useMemo(() => {
    switch (camera.streamMode) {
      case 'main':
        return t('manage.streamModeMainBadge', { defaultValue: '主码流' })
      case 'sub':
        return t('manage.streamModeSubBadge', { defaultValue: '子码流' })
      default:
        return t('manage.streamModeAutoBadge', { defaultValue: '自动' })
    }
  }, [camera.streamMode, t])

  // 探活操作按钮样式与文本推导
  let probeBtnStyle =
    'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)] hover:border-[var(--accent)]/40 hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]'
  let probeLabel = t('manage.probeAction', { defaultValue: '即时探活' })

  if (isProbing) {
    probeLabel = t('manage.probing', { defaultValue: '探活中...' })
  } else if (probeFeedback === 'success') {
    probeBtnStyle = 'border-emerald-500/35 bg-emerald-500/10 text-emerald-400 shadow-xs'
    probeLabel = t('manage.probeSuccess', { defaultValue: '探活成功' })
  } else if (probeFeedback === 'failed') {
    probeBtnStyle = 'border-rose-500/35 bg-rose-500/10 text-rose-400 shadow-xs'
    probeLabel = t('manage.probeFailed', { defaultValue: '探活失败' })
  }

  return (
    <motion.div
      layout
      initial={{ opacity: 0, scale: 0.98 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.98 }}
      transition={{
        duration: motionTokens.duration.fast,
        ease: motionTokens.easing.smooth,
      }}
      className={`frosted-glass group relative flex flex-col justify-between overflow-hidden rounded-2xl border p-4 transition-all duration-200 hover:-translate-y-0.5 sm:p-4.5 ${statusBorderStyle}`}
    >
      {/* 顶部微米级状态高光线 */}
      <div
        className={`pointer-events-none absolute top-0 right-0 left-0 h-[1.5px] opacity-70 transition-opacity duration-300 group-hover:opacity-100 ${statusGlowStyle}`}
      />

      {/* ── 1. 顶部身份与状态标牌 ── */}
      <div>
        <div className="flex items-start justify-between gap-3">
          {/* 设备名称与标识 */}
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h3
                className="truncate text-base font-bold tracking-tight text-[var(--text-primary)] transition-colors group-hover:text-[var(--accent)]"
                title={camera.name || camera.cameraId}
              >
                {camera.name || camera.cameraId}
              </h3>
              <span className="shrink-0 rounded-md border border-[var(--border)] bg-[var(--bg-secondary)] px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[var(--text-secondary)]">
                #{camera.cameraId}
              </span>
            </div>

            {/* 备注或接入描述 */}
            <div className="mt-1 flex items-center gap-1.5 text-xs text-[var(--text-muted)]">
              {camera.remark ? (
                <>
                  <FileText className="h-3 w-3 shrink-0 opacity-70" />
                  <span className="truncate" title={camera.remark}>
                    {camera.remark}
                  </span>
                </>
              ) : (
                <span className="font-mono text-[11px] text-[var(--text-muted)] opacity-70">
                  {isGb ? 'GB/T 28181 IPC' : 'RTSP STREAM'}
                </span>
              )}
            </div>
          </div>

          {/* 右侧状态微标 + 协议胶囊 */}
          <div className="flex shrink-0 items-center gap-1.5">
            <span
              className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 font-mono text-[11px] font-semibold backdrop-blur-xs ${probeBadge.badgeBg}`}
            >
              <span
                className={`h-1.5 w-1.5 rounded-full ${probeBadge.dotClass} ${
                  normalizedStatus === 'healthy' ? 'animate-pulse' : ''
                }`}
              />
              <span>{probeBadge.text}</span>
            </span>

            <span className="rounded-md border border-[var(--border)] bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[10px] font-bold text-[var(--text-secondary)] uppercase shadow-2xs">
              {isGb ? 'GB28181' : 'RTSP'}
            </span>
          </div>
        </div>

        {/* ── 2. 串流信源端点 (Terminal Stream Capsule) ── */}
        <div className="mt-3.5 flex items-center justify-between gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/50 px-2.5 py-1.5 text-xs backdrop-blur-xs transition-colors hover:border-[var(--border-strong)]">
          <div className="flex min-w-0 items-center gap-2">
            <span className="shrink-0 rounded bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[9px] font-bold text-[var(--text-muted)] uppercase">
              {isGb ? 'SIP' : 'URI'}
            </span>
            <span
              className="truncate font-mono text-[11px] text-[var(--text-secondary)] select-all"
              title={camera.rtspUrl}
            >
              {camera.rtspUrl}
            </span>
          </div>

          <button
            type="button"
            onClick={() => onCopyRtsp(camera.cameraId, camera.rtspUrl)}
            title={isCopied ? t('manage.copied') : t('manage.copyUrl')}
            className={`flex shrink-0 items-center gap-1 rounded-md px-1.5 py-0.5 text-xs font-medium transition-all ${
              isCopied
                ? 'bg-emerald-500/15 text-emerald-400'
                : 'text-[var(--text-muted)] hover:bg-[var(--bg-surface)] hover:text-[var(--accent)]'
            }`}
          >
            {isCopied ? (
              <>
                <Check className="h-3 w-3 text-emerald-400" />
                <span className="text-[10px] font-semibold text-emerald-400">
                  {t('manage.copied')}
                </span>
              </>
            ) : (
              <Copy className="h-3.5 w-3.5" />
            )}
          </button>
        </div>

        {/* ── 3. 规格遥测轻量贯穿仪表条 (Horizontal Specs Strip) ── */}
        <div className="mt-3 grid grid-cols-3 divide-x divide-[var(--border)]/70 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/35 p-2.5">
          {/* 编码格式 */}
          <div className="flex flex-col justify-between pr-2.5">
            <div className="flex items-center gap-1 text-[10px] text-[var(--text-muted)]">
              <Film className="h-3 w-3 opacity-70" />
              <span>{t('manage.codec')}</span>
            </div>
            <div className="mt-1.5 flex items-center gap-1">
              <span className="truncate font-mono text-xs font-bold text-[var(--text-primary)] uppercase">
                {camera.lastCodec || '—'}
              </span>
              {isH265 && (
                <span
                  title={t('manage.vpuHwReady', { defaultValue: '硬件解码加速就绪' })}
                  className="py-0.2 flex items-center gap-0.5 rounded border border-[var(--border)] bg-[var(--bg-surface)] px-1 font-mono text-[8px] font-bold text-[var(--accent)] uppercase"
                >
                  <Cpu className="h-2 w-2" />
                  <span>HW</span>
                </span>
              )}
            </div>
          </div>

          {/* 分辨率与帧率 */}
          <div className="flex flex-col justify-between px-2.5">
            <div className="flex items-center gap-1 text-[10px] text-[var(--text-muted)]">
              <Maximize2 className="h-3 w-3 opacity-70" />
              <span>{t('manage.resolutionFps')}</span>
            </div>
            <div className="mt-1.5 flex items-center gap-1 truncate font-mono text-xs font-semibold text-[var(--text-primary)] tabular-nums">
              {camera.lastWidth && camera.lastHeight ? (
                <>
                  {resTier && (
                    <span className="py-0.2 rounded border border-[var(--border)] bg-[var(--bg-surface)] px-1 font-mono text-[8px] font-bold text-[var(--accent)]">
                      {resTier}
                    </span>
                  )}
                  <span className="truncate">
                    {camera.lastWidth}×{camera.lastHeight}
                  </span>
                  {camera.lastFps ? (
                    <span className="font-bold text-emerald-500">{camera.lastFps.toFixed(0)}F</span>
                  ) : null}
                </>
              ) : (
                <span className="font-normal text-[var(--text-muted)]">
                  {t('manage.pendingProbeHint', { defaultValue: '待探活' })}
                </span>
              )}
            </div>
          </div>

          {/* 分析码流策略 */}
          <div className="flex flex-col justify-between pl-2.5">
            <div className="flex items-center gap-1 text-[10px] text-[var(--text-muted)]">
              <Split className="h-3 w-3 opacity-70" />
              <span>{t('manage.analysisStream')}</span>
            </div>
            <div className="mt-1.5 truncate text-xs font-semibold">
              <span className="font-bold text-[var(--accent)]">{streamModeLabel}</span>
            </div>
          </div>
        </div>

        {/* ── 4. AI 任务关联状态与布防跳转 ── */}
        <div className="mt-3 flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/50 px-3 py-2 text-xs backdrop-blur-xs">
          <div className="flex min-w-0 items-center gap-2">
            {boundTask ? (
              <>
                <ShieldCheck className="h-4 w-4 shrink-0 text-emerald-500" />
                <div className="flex min-w-0 flex-col">
                  <span className="truncate font-semibold text-[var(--text-primary)]">
                    {t('manage.aiTaskBound', { defaultValue: '已配置布防任务' })}
                  </span>
                  <span className="font-mono text-[10px] text-[var(--text-muted)]">
                    {t('manage.rulesSummary', {
                      count: boundTask.rulesCount,
                      defaultValue: `${boundTask.rulesCount} 项空间几何规则`,
                    })}{' '}
                    ·{' '}
                    {boundTask.desiredEnabled
                      ? t('manage.armedStatus', { defaultValue: '布防中' })
                      : t('manage.disarmedStatus', { defaultValue: '未布防' })}
                  </span>
                </div>
              </>
            ) : (
              <>
                <ShieldAlert className="h-4 w-4 shrink-0 text-[var(--text-muted)]" />
                <span className="truncate text-[var(--text-muted)]">
                  {t('manage.aiTaskUnbound', { defaultValue: '未分配 AI 任务' })}
                </span>
              </>
            )}
          </div>

          {onNavigateToTasks && (
            <button
              type="button"
              onClick={() => onNavigateToTasks(camera)}
              className="shrink-0 text-xs font-semibold text-[var(--accent)] transition-colors hover:underline"
            >
              {boundTask
                ? t('manage.goToTask', { defaultValue: '前往布防' })
                : t('manage.createTask', { defaultValue: '创建布防' })}
            </button>
          )}
        </div>
      </div>

      {/* ── 5. 底部动作与探活时效栏 (Footer Actions & Freshness) ── */}
      <div className="mt-4 flex items-center justify-between border-t border-[var(--border)] pt-3">
        {/* 左侧：探活时效指示 */}
        <div className="flex items-center gap-1.5 font-mono text-[11px] text-[var(--text-muted)]">
          <Clock className="h-3.5 w-3.5 opacity-60" />
          <span>{probeFreshness}</span>
        </div>

        {/* 右侧：动作按钮组 */}
        <div className="flex items-center gap-1.5">
          {/* 单机即时探活 */}
          <button
            type="button"
            onClick={() => onManualProbe(camera)}
            disabled={isProbing}
            className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs font-medium transition-all ${probeBtnStyle} active:scale-95 disabled:opacity-50`}
          >
            <RefreshCw
              className={`h-3 w-3 ${isProbing ? 'animate-spin text-[var(--accent)]' : ''}`}
            />
            <span>{probeLabel}</span>
          </button>

          {/* 编辑配置 */}
          <button
            type="button"
            onClick={() => onEdit(camera)}
            title={tc('actions.edit')}
            className="flex items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)]/40 hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] active:scale-95"
          >
            <Pencil className="h-3 w-3" />
            <span>{tc('actions.edit')}</span>
          </button>

          {/* 删除设备 */}
          <button
            type="button"
            onClick={() => onDelete(camera)}
            title={tc('actions.delete')}
            className="flex h-7 w-7 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)] transition-colors hover:border-[var(--destructive)]/40 hover:bg-[var(--destructive)]/10 hover:text-[var(--destructive)] active:scale-95"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    </motion.div>
  )
}
