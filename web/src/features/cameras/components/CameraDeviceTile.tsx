import React, { useEffect, useRef, useState } from 'react'
import { Check, Copy, Eye, MoreHorizontal, Pencil, RefreshCw, Trash2 } from 'lucide-react'
import { motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { copyToClipboard } from '@/lib/utils'
import type { Camera } from '@/types'
import {
  getCameraAiRuntimeStatus,
  getProbeBadge,
  getProbeButtonText,
  normalizeProbeStatus,
  type CameraAiRuntimeStatus,
  type CameraTaskRuntimeView,
} from '../cameraStatus'
import { CameraIllustration } from './illustrations/CameraIllustration'
import {
  getCameraTypeLabel,
  getSavedCameraModelType,
  resolveCameraModelType,
} from './illustrations/cameraModelType'
import type { CameraModelType, CameraOperationalStatus } from './illustrations/types'

export interface CameraDeviceTileProps {
  camera: Camera
  task?: CameraTaskRuntimeView | null
  cameraType?: CameraModelType
  isProbing?: boolean
  probeFeedback?: 'success' | 'failed'
  onManualProbe?: (camera: Camera) => void
  onEdit?: (camera: Camera) => void
  onDelete?: (camera: Camera) => void
  onClick?: (camera: Camera) => void
  onDetail?: (camera: Camera) => void
  defaultMenuOpen?: boolean
  className?: string
}

const AI_STATUS_STYLES: Record<CameraAiRuntimeStatus, string> = {
  active: 'text-slate-800 dark:text-slate-200',
  starting: 'text-amber-500',
  degraded: 'text-amber-500',
  error: 'text-[var(--status-danger)]',
  inactive: 'text-[var(--text-muted)]',
}

const AI_STATUS_FALLBACKS: Record<CameraAiRuntimeStatus, string> = {
  active: 'AI Active',
  starting: 'Starting',
  degraded: 'Degraded',
  error: 'Error',
  inactive: 'Inactive',
}

function getResolutionTier(width: number, height: number): string | null {
  if (width >= 3840 || height >= 2160) return '4K'
  if (width >= 2560 || height >= 1440) return '2K'
  if (width >= 1920 || height >= 1080) return '1080P'
  if (width >= 1280 || height >= 720) return '720P'
  return null
}

function formatResolution(
  camera: Camera,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  if (!camera.lastWidth || !camera.lastHeight) {
    return t('tile.unknownValue', { defaultValue: '—' })
  }

  const { lastWidth: width, lastHeight: height } = camera
  const tier = getResolutionTier(width, height)

  return tier ? `${tier} · ${width}×${height}` : `${width}×${height}`
}

export function CameraDeviceTile({
  camera,
  task,
  cameraType,
  isProbing = false,
  probeFeedback,
  onManualProbe,
  onEdit,
  onDelete,
  onClick,
  onDetail,
  defaultMenuOpen = false,
  className = '',
}: CameraDeviceTileProps): React.ReactElement {
  const { t } = useTranslation('camera')
  const [menuOpen, setMenuOpen] = useState(defaultMenuOpen)
  const [copied, setCopied] = useState(false)
  const menuRef = useRef<HTMLDivElement | null>(null)
  const copiedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    if (!menuOpen) return

    const handleClickOutside = (event: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        setMenuOpen(false)
      }
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setMenuOpen(false)
    }

    document.addEventListener('mousedown', handleClickOutside)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('mousedown', handleClickOutside)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [menuOpen])

  useEffect(
    () => () => {
      if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
    },
    [],
  )

  const modelType =
    cameraType ?? getSavedCameraModelType(camera.cameraId) ?? resolveCameraModelType(camera)

  const normalizedProbe = normalizeProbeStatus(camera.lastProbeStatus)
  const operationalStatus: CameraOperationalStatus =
    normalizedProbe === 'healthy'
      ? 'online'
      : normalizedProbe === 'degraded'
        ? 'warning'
        : 'offline'
  const probeBadge = getProbeBadge(camera.lastProbeStatus, t)
  const aiRuntime = getCameraAiRuntimeStatus(task)
  const aiStatusLabel = t(`tile.aiStatus.${aiRuntime.status}`, {
    defaultValue: AI_STATUS_FALLBACKS[aiRuntime.status],
  })

  const handleTileClick = () => {
    // 点击卡片主体：默认打开【详情】抽屉
    if (onClick) {
      onClick(camera)
    } else if (onDetail) {
      onDetail(camera)
    } else if (onEdit) {
      onEdit(camera)
    }
  }

  const handleTileKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.target !== event.currentTarget) return
    if ((!onEdit && !onClick && !onDetail) || (event.key !== 'Enter' && event.key !== ' ')) return
    event.preventDefault()
    handleTileClick()
  }

  const handleCopy = async (event: React.MouseEvent) => {
    event.stopPropagation()
    const success = await copyToClipboard(camera.rtspUrl)
    if (!success) return

    setCopied(true)
    if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
    copiedTimerRef.current = setTimeout(() => {
      setCopied(false)
      setMenuOpen(false)
    }, 1200)
  }

  const handleModelLabel = getCameraTypeLabel(modelType, t)

  // 格式化遥测数据：1920 × 1080 · 25 FPS
  const resText = formatResolution(camera, t)
  const fpsText = camera.lastFps ? `${camera.lastFps.toFixed(0)} FPS` : ''
  const telemetryParts = [resText !== '—' ? resText : null, fpsText || null].filter(Boolean)
  const telemetryText =
    telemetryParts.length > 0
      ? telemetryParts.join(' · ')
      : t('tile.unknownValue', { defaultValue: '—' })

  const instancesCount = task?.algorithmInstances?.length || 0
  const aiDisplayLabel =
    aiRuntime.status === 'active' && instancesCount > 1 ? `AI · ${instancesCount}` : aiStatusLabel

  const hasAction = Boolean(onEdit || onClick)

  return (
    <motion.article
      layout
      initial={{ opacity: 0, scale: 0.98 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.98 }}
      transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
      tabIndex={hasAction ? 0 : undefined}
      title={
        onClick
          ? t('tile.viewDetail', {
              name: camera.name || camera.cameraId,
              defaultValue: `详情 · ${camera.name || camera.cameraId}`,
            })
          : onEdit
            ? `${t('tile.editAction', { defaultValue: '编辑设备参数' })}: ${camera.name || camera.cameraId}`
            : undefined
      }
      aria-label={
        onClick
          ? t('tile.openDetail', {
              name: camera.name || camera.cameraId,
              defaultValue: `详情 · ${camera.name || camera.cameraId}`,
            })
          : onEdit
            ? `${t('tile.editAction', { defaultValue: '编辑设备参数' })}: ${camera.name || camera.cameraId}`
            : undefined
      }
      onClick={handleTileClick}
      onKeyDown={handleTileKeyDown}
      className={`group relative flex min-w-0 flex-col justify-between overflow-hidden rounded-[26px] border border-[var(--border)] bg-white text-left transition-all duration-200 hover:-translate-y-0.5 hover:shadow-md dark:bg-[var(--bg-surface-solid)] ${hasAction ? 'cursor-pointer' : ''} ${className}`}
    >
      {/* ── 1. 硬件展示舞台区 (Top Stage) ── */}
      <div className="relative flex min-h-[175px] flex-col justify-between bg-[var(--bg-secondary)]/50 p-4 transition-colors sm:min-h-[190px] dark:bg-[var(--bg-secondary)]/80">
        {/* 顶部悬浮控制栏：状态胶囊 + 唯一更多操作入口 (...) */}
        <div className="relative z-10 flex items-center justify-between gap-2">
          {/* 状态药丸 */}
          <div className="inline-flex items-center gap-1.5 rounded-full border border-black/[0.05] bg-white/95 px-3 py-1 text-[11px] font-semibold tracking-tight text-[var(--text-secondary)] shadow-xs backdrop-blur-md dark:border-white/[0.08] dark:bg-black/50">
            <span
              aria-hidden="true"
              className={`h-2 w-2 shrink-0 rounded-full ${probeBadge.dotClass} ${operationalStatus === 'online' ? 'animate-pulse' : ''}`}
            />
            <span className="truncate">{probeBadge.text}</span>
          </div>

          {/* 右侧唯一更多操作按钮 (...) 与下拉菜单 */}
          <div className="relative shrink-0" ref={menuRef}>
            <button
              type="button"
              aria-label={t('tile.moreActions', { defaultValue: '更多操作' })}
              aria-expanded={menuOpen}
              aria-haspopup="menu"
              title={t('tile.moreActions', { defaultValue: '更多操作' })}
              onClick={(event) => {
                event.stopPropagation()
                setMenuOpen((open) => !open)
              }}
              className="flex h-8 w-8 items-center justify-center rounded-xl border border-black/[0.05] bg-white/95 text-[var(--text-muted)] shadow-xs backdrop-blur-md transition-colors hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none dark:border-white/[0.08] dark:bg-black/50"
            >
              <MoreHorizontal className="h-4 w-4" />
            </button>

            {menuOpen && (
              <div
                role="menu"
                className="lens-glass absolute top-full right-0 z-50 mt-1.5 min-w-[168px] overflow-hidden rounded-2xl border border-[var(--border)] p-1.5 shadow-xl"
                onClick={(event) => event.stopPropagation()}
              >
                {/* 编辑设备参数 - 最高优先级置顶 */}
                {onEdit && (
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setMenuOpen(false)
                      onEdit(camera)
                    }}
                    className="flex min-h-9 w-full items-center gap-2 rounded-xl px-2.5 text-xs font-semibold text-[var(--text-primary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                  >
                    <Pencil className="h-3.5 w-3.5 opacity-80" />
                    <span>{t('tile.editAction', { defaultValue: '编辑设备参数' })}</span>
                  </button>
                )}

                {/* 查看设备详情 */}
                {(onDetail || onClick) && (
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setMenuOpen(false)
                      if (onDetail) {
                        onDetail(camera)
                      } else if (onClick) {
                        onClick(camera)
                      }
                    }}
                    className="flex min-h-9 w-full items-center gap-2 rounded-xl px-2.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                  >
                    <Eye className="h-3.5 w-3.5 opacity-70" />
                    <span>{t('tile.viewDetailAction', { defaultValue: '查看设备详情' })}</span>
                  </button>
                )}

                {/* 即时探活 */}
                {onManualProbe && (
                  <button
                    type="button"
                    role="menuitem"
                    disabled={isProbing}
                    onClick={() => {
                      setMenuOpen(false)
                      onManualProbe(camera)
                    }}
                    className="flex min-h-9 w-full items-center gap-2 rounded-xl px-2.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
                  >
                    <RefreshCw
                      className={`h-3.5 w-3.5 opacity-70 ${isProbing ? 'animate-spin' : ''}`}
                    />
                    <span>
                      {getProbeButtonText(
                        isProbing,
                        probeFeedback,
                        t,
                        'tile.probeAction',
                        '即时探活',
                      )}
                    </span>
                  </button>
                )}

                {/* 复制主流地址 */}
                {camera.rtspUrl && (
                  <button
                    type="button"
                    role="menuitem"
                    onClick={handleCopy}
                    className="flex min-h-9 w-full items-center gap-2 rounded-xl px-2.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                  >
                    {copied ? (
                      <Check className="h-3.5 w-3.5 text-emerald-500" />
                    ) : (
                      <Copy className="h-3.5 w-3.5 opacity-70" />
                    )}
                    <span>
                      {copied
                        ? t('manage.copied', { defaultValue: '已复制' })
                        : t('tile.copyAction', { defaultValue: '复制主流地址' })}
                    </span>
                  </button>
                )}

                {/* 移除设备 */}
                {onDelete && (
                  <>
                    <div className="my-1 border-t border-[var(--border)]" />
                    <button
                      type="button"
                      role="menuitem"
                      onClick={() => {
                        setMenuOpen(false)
                        onDelete(camera)
                      }}
                      className="flex min-h-9 w-full items-center gap-2 rounded-xl px-2.5 text-xs font-medium text-[var(--status-danger)] transition-colors hover:bg-[var(--status-danger-soft)] focus-visible:ring-2 focus-visible:ring-[var(--status-danger)]/40 focus-visible:outline-none"
                    >
                      <Trash2 className="h-3.5 w-3.5 opacity-80" />
                      <span>{t('tile.deleteAction', { defaultValue: '移除设备' })}</span>
                    </button>
                  </>
                )}
              </div>
            )}
          </div>
        </div>

        {/* 现代极简硬件矢量插画 */}
        <div className="relative flex flex-1 items-center justify-center py-2 transition-transform duration-200 ease-out group-hover:scale-105">
          <CameraIllustration
            type={modelType}
            status={operationalStatus}
            aiActive={aiRuntime.isActive}
            camera={camera}
            className="camera-illustration h-28 w-auto max-w-[180px] sm:h-32"
            ariaLabel={t('tile.illustrationLabel', {
              type: handleModelLabel,
              defaultValue: `${handleModelLabel} illustration`,
            })}
          />
        </div>
      </div>

      {/* ── 2. 用户可见信息区 (UserInfo Area) ── */}
      <div className="flex flex-col justify-between bg-white p-4 sm:p-5 dark:bg-[var(--bg-surface-solid)]">
        <div>
          <h3
            title={camera.name || camera.cameraId}
            className="truncate text-[15px] font-semibold tracking-tight text-[var(--text-primary)] transition-colors group-hover:text-[var(--accent)]"
          >
            {camera.name || camera.cameraId}
          </h3>
          <p
            className="mt-0.5 truncate text-xs text-[var(--text-muted)]"
            title={camera.remark || undefined}
          >
            {camera.remark || t('tile.locationUnset', { defaultValue: '—' })}
          </p>
        </div>

        {/* 极细分隔线 */}
        <div className="my-3 border-t border-[var(--border)]/70" />

        {/* 底部遥测与 AI 状态单行读数 */}
        <div className="flex items-center justify-between gap-2 text-xs">
          <span className="font-data truncate text-[11.5px] font-medium tracking-tight text-[var(--text-secondary)] tabular-nums">
            {telemetryText}
          </span>

          <span
            title={aiStatusLabel}
            aria-label={aiStatusLabel}
            className={`flex shrink-0 items-center gap-1.5 text-[11px] font-semibold tracking-tight ${AI_STATUS_STYLES[aiRuntime.status]}`}
          >
            <span
              aria-hidden="true"
              className={`h-1.5 w-1.5 rounded-full bg-current ${aiRuntime.status === 'active' ? 'animate-pulse' : ''}`}
            />
            <span>{aiDisplayLabel}</span>
          </span>
        </div>
      </div>
    </motion.article>
  )
}
