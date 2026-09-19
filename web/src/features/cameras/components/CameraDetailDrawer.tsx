import React, { useEffect, useId, useMemo, useState } from 'react'
import {
  Check,
  Clock3,
  Copy,
  Cpu,
  FileText,
  Layers3,
  MapPin,
  Maximize2,
  Pencil,
  RefreshCw,
  Split,
  Trash2,
  Video,
  X,
} from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { motionTokens } from '@/lib/motionTokens'
import { formatRelativeTime, formatTimestamp } from '@/lib/time'
import { copyToClipboard } from '@/lib/utils'
import type { Camera } from '@/types'
import {
  getActiveAlgorithmIds,
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
  saveCameraModelType,
} from './illustrations/cameraModelType'
import type { CameraModelType, CameraOperationalStatus } from './illustrations/types'

export interface CameraDetailDrawerProps {
  camera: Camera | null
  task?: CameraTaskRuntimeView | null
  onClose: () => void
  onManualProbe?: (camera: Camera) => void
  onEdit?: (camera: Camera) => void
  onDelete?: (camera: Camera) => void
  onModelTypeChange?: (cameraId: string, type: CameraModelType) => void
  isProbing?: boolean
  probeFeedback?: 'success' | 'failed'
}

const PIPELINE_STATUS_STYLES = {
  active: 'text-cyan-500 dark:text-cyan-400 bg-cyan-500/10 border-cyan-500/20',
  starting: 'text-amber-500 dark:text-amber-400 bg-amber-500/10 border-amber-500/20',
  degraded: 'text-amber-500 dark:text-amber-400 bg-amber-500/10 border-amber-500/20',
  error: 'text-rose-500 dark:text-rose-400 bg-rose-500/10 border-rose-500/20',
  inactive: 'text-[var(--text-muted)] bg-[var(--bg-secondary)] border-[var(--border)]',
} as const

const PIPELINE_STATUS_FALLBACKS: Record<CameraAiRuntimeStatus, string> = {
  active: 'Running',
  starting: 'Starting',
  degraded: 'Degraded',
  error: 'Error',
  inactive: 'Inactive',
}

const CONNECTION_STATUS_CLASSES: Record<CameraOperationalStatus, string> = {
  online: 'text-emerald-500 bg-emerald-500/10 border-emerald-500/20',
  warning: 'text-amber-500 bg-amber-500/10 border-amber-500/20',
  offline: 'text-slate-500 bg-slate-500/10 border-slate-500/20',
}

function getPipelineStatusLabel(
  status: CameraAiRuntimeStatus,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  return t(`drawer.pipeline.${status}`, {
    defaultValue: PIPELINE_STATUS_FALLBACKS[status] ?? 'Inactive',
  })
}

function getStreamModeLabel(
  mode: string | undefined,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  switch (mode) {
    case 'main':
      return t('manage.streamModeMainBadge', { defaultValue: '主码流分析' })
    case 'sub':
      return t('manage.streamModeSubBadge', { defaultValue: '子码流分析' })
    default:
      return t('manage.streamModeAutoBadge', { defaultValue: '自动码流' })
  }
}

export function CameraDetailDrawer({
  camera,
  task,
  onClose,
  onManualProbe,
  onEdit,
  onDelete,
  onModelTypeChange,
  isProbing = false,
  probeFeedback,
}: CameraDetailDrawerProps): React.ReactElement | null {
  const { t, i18n } = useTranslation('camera')
  const { t: tc } = useTranslation('common')
  const reducedMotion = useReducedMotion()
  const titleId = useId()
  const [copiedKey, setCopiedKey] = useState<string | null>(null)
  const [modelType, setModelType] = useState<CameraModelType>('bullet')
  const [activeCamera, setActiveCamera] = useState<Camera | null>(camera)
  const [activeTask, setActiveTask] = useState<CameraTaskRuntimeView | null | undefined>(task)

  const isOpen = Boolean(camera)
  useDismissStack(isOpen, onClose)

  useEffect(() => {
    if (camera) {
      setActiveCamera(camera)
      setActiveTask(task)
      const saved = getSavedCameraModelType(camera.cameraId)
      setModelType(saved ?? resolveCameraModelType(camera))
      setCopiedKey(null)
    }
  }, [camera, task])

  const currentCamera = camera ?? activeCamera
  const currentTask = camera ? task : (activeTask ?? task)

  const handleModelSelect = (nextType: CameraModelType) => {
    setModelType(nextType)
    if (currentCamera) {
      saveCameraModelType(currentCamera.cameraId, nextType)
      onModelTypeChange?.(currentCamera.cameraId, nextType)
    }
  }

  const handleCopy = async (key: string, value: string) => {
    if (!value) return
    if (await copyToClipboard(value)) {
      setCopiedKey(key)
      window.setTimeout(() => setCopiedKey(null), 2000)
    }
  }

  const derived = useMemo(() => {
    if (!currentCamera) return null

    const normalizedProbe = normalizeProbeStatus(currentCamera.lastProbeStatus)
    const operationalStatus: CameraOperationalStatus =
      normalizedProbe === 'healthy'
        ? 'online'
        : normalizedProbe === 'degraded'
          ? 'warning'
          : 'offline'
    const aiRuntime = getCameraAiRuntimeStatus(currentTask)
    const activityTimes = [
      currentCamera.lastProbeAt,
      currentCamera.lastSuccessAt,
      currentTask?.updatedAt,
    ].filter((value): value is number => typeof value === 'number' && value > 0)
    const lastActivityAt = activityTimes.length > 0 ? Math.max(...activityTimes) : null

    return {
      operationalStatus,
      aiRuntime,
      lastActivityAt,
      probeBadge: getProbeBadge(currentCamera.lastProbeStatus, t),
      activeAlgorithmIds: getActiveAlgorithmIds(currentTask),
    }
  }, [currentCamera, currentTask, t])

  const typeLabel = getCameraTypeLabel(modelType, t)
  const pipelineStatusLabel = derived ? getPipelineStatusLabel(derived.aiRuntime.status, t) : ''
  const lastActivityLabel = derived?.lastActivityAt
    ? formatRelativeTime(derived.lastActivityAt, i18n.language)
    : t('drawer.noActivity', { defaultValue: 'No recorded activity' })
  const lastActivityTimestamp = derived?.lastActivityAt
    ? formatTimestamp(derived.lastActivityAt)
    : t('tile.unknownValue', { defaultValue: '—' })
  const streamModeLabel = getStreamModeLabel(currentCamera?.streamMode, t)
  const activeInstances = (derived?.activeAlgorithmIds ?? []).map((algorithmId) =>
    currentTask?.algorithmInstances?.find(
      (instance) => instance.enabled && instance.algorithmId.trim() === algorithmId,
    ),
  )
  const connectionStatusClass = CONNECTION_STATUS_CLASSES[derived?.operationalStatus ?? 'offline']

  return (
    <AnimatePresence
      onExitComplete={() => {
        if (!camera) {
          setActiveCamera(null)
          setActiveTask(null)
        }
      }}
    >
      {isOpen && currentCamera && derived && (
        <div className="fixed inset-0 z-50 flex justify-end">
          {/* 背景遮罩 */}
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: motionTokens.duration.fast }}
            onClick={onClose}
            className="fixed inset-0 bg-black/60 backdrop-blur-xs"
            aria-hidden="true"
          />

          {/* 抽屉主面板 */}
          <motion.aside
            role="dialog"
            aria-modal="true"
            aria-labelledby={titleId}
            initial={reducedMotion ? false : { x: '100%' }}
            animate={{ x: 0 }}
            exit={reducedMotion ? { opacity: 0 } : { x: '100%' }}
            transition={{ type: 'spring', damping: 28, stiffness: 300 }}
            className="relative z-10 flex h-full w-full max-w-xl flex-col border-l border-[var(--border)] bg-white shadow-2xl dark:bg-[var(--bg-surface-solid)]"
          >
            {/* ── 1. 顶部标题栏 (紧凑高密度现代 SaaS 抽屉头) ── */}
            <header className="flex shrink-0 items-center justify-between border-b border-[var(--border)]/70 px-5 py-3 sm:px-6">
              <div className="flex min-w-0 items-center gap-3">
                <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-[var(--accent)]/20 bg-[var(--accent-soft)] text-[var(--accent)] shadow-xs">
                  <Video className="h-5 w-5" />
                </div>
                <div className="min-w-0">
                  {/* 第一行：设备名称 + 可点按复制的 ID 徽标 + 协议胶囊 (同行紧凑排布) */}
                  <div className="flex flex-wrap items-center gap-2">
                    <h2
                      id={titleId}
                      title={currentCamera.name || currentCamera.cameraId}
                      className="truncate text-base font-bold tracking-tight text-[var(--text-primary)]"
                    >
                      {currentCamera.name || currentCamera.cameraId}
                    </h2>

                    {/* 可点按复制的 ID 胶囊（同行并排） */}
                    <button
                      type="button"
                      onClick={() => void handleCopy('id', currentCamera.cameraId)}
                      title={
                        copiedKey === 'id'
                          ? tc('actions.copied', { defaultValue: '已复制' })
                          : t('tile.copyId', { defaultValue: '点击复制设备 ID' })
                      }
                      aria-label={
                        copiedKey === 'id'
                          ? tc('actions.copied', { defaultValue: '已复制' })
                          : t('tile.copyId', { defaultValue: '点击复制设备 ID' })
                      }
                      className="group/id font-data inline-flex items-center gap-1 rounded-md border border-[var(--border)]/80 bg-[var(--bg-secondary)]/50 px-1.5 py-0.5 text-[11px] text-[var(--text-secondary)] transition-all hover:border-[var(--accent)] hover:bg-white hover:text-[var(--accent)] active:scale-95 dark:hover:bg-[var(--bg-surface-solid)]"
                    >
                      <span className="text-[10px] text-[var(--text-muted)] group-hover/id:text-[var(--accent)]">
                        ID:
                      </span>
                      <span className="font-semibold">{currentCamera.cameraId}</span>
                      {copiedKey === 'id' ? (
                        <Check className="h-2.5 w-2.5 text-emerald-500" />
                      ) : (
                        <Copy className="h-2.5 w-2.5 opacity-60 group-hover/id:opacity-100" />
                      )}
                    </button>

                    {/* 协议胶囊 */}
                    <span className="rounded-md border border-[var(--border)] bg-[var(--bg-secondary)] px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[var(--text-muted)]">
                      {currentCamera.protocol.toUpperCase()}
                    </span>
                  </div>

                  {/* 第二行：设备详情标识 · 点位安装备注 (紧凑小字) */}
                  <div className="mt-0.5 flex items-center gap-1.5 text-xs text-[var(--text-muted)]">
                    <span>{t('drawer.title', { defaultValue: '设备详情' })}</span>
                    {currentCamera.remark && (
                      <>
                        <span>·</span>
                        <span className="flex items-center gap-1 truncate text-[var(--text-secondary)]">
                          <MapPin className="h-3 w-3 shrink-0 opacity-70" />
                          <span className="truncate">{currentCamera.remark}</span>
                        </span>
                      </>
                    )}
                  </div>
                </div>
              </div>

              <button
                type="button"
                onClick={onClose}
                aria-label={t('drawer.close', { defaultValue: '关闭 (Esc)' })}
                title={t('drawer.close', { defaultValue: '关闭 (Esc)' })}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
              >
                <X className="h-4 w-4" />
              </button>
            </header>

            {/* ── 2. 工具栏：编辑（主操作置顶） / 探活 / 删除 ── */}
            <div className="flex shrink-0 items-center justify-between border-b border-[var(--border)]/60 bg-[var(--bg-secondary)]/20 px-5 py-2.5 sm:px-6">
              <div className="flex items-center gap-2">
                {onEdit && (
                  <button
                    type="button"
                    onClick={() => onEdit(currentCamera)}
                    className="flex min-h-8.5 items-center gap-1.5 rounded-xl bg-slate-900 px-3.5 text-xs font-semibold text-white shadow-xs transition-colors hover:bg-slate-800 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none dark:bg-slate-100 dark:text-slate-900 dark:hover:bg-white"
                  >
                    <Pencil className="h-3.5 w-3.5" />
                    <span>{tc('actions.edit')}</span>
                  </button>
                )}
                {onManualProbe && (
                  <button
                    type="button"
                    onClick={() => onManualProbe(currentCamera)}
                    disabled={isProbing}
                    className="flex min-h-8.5 items-center gap-1.5 rounded-xl border border-[var(--border)]/80 bg-white px-3 text-xs font-medium text-[var(--text-secondary)] shadow-xs transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50 dark:bg-[var(--bg-surface-solid)]"
                  >
                    <RefreshCw className={`h-3.5 w-3.5 ${isProbing ? 'animate-spin' : ''}`} />
                    <span>{getProbeButtonText(isProbing, probeFeedback, t)}</span>
                  </button>
                )}
              </div>
              {onDelete && (
                <button
                  type="button"
                  onClick={() => onDelete(currentCamera)}
                  className="flex min-h-8.5 items-center gap-1.5 rounded-xl px-2.5 text-xs font-medium text-rose-500 transition-colors hover:bg-rose-500/10 focus-visible:ring-2 focus-visible:ring-rose-500/40 focus-visible:outline-none"
                >
                  <Trash2 className="h-3.5 w-3.5 opacity-80" />
                  <span>{tc('actions.delete')}</span>
                </button>
              )}
            </div>

            {/* ── 3. 主体滚动区域 ── */}
            <div className="flex-1 space-y-5 overflow-y-auto px-5 py-5 sm:px-6">
              {/* ── 舞台展示卡片 ── */}
              <div className="relative flex flex-col items-center justify-between overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)]/50 p-5 dark:bg-[var(--bg-secondary)]/70">
                {/* 舞台顶栏：实时在线状态胶囊 + 模型切换器 */}
                <div className="relative z-10 flex w-full items-center justify-between gap-2">
                  <div
                    className={`inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-xs font-medium ${connectionStatusClass}`}
                  >
                    <span
                      className={`h-2 w-2 rounded-full bg-current ${derived.operationalStatus === 'online' ? 'animate-pulse' : ''}`}
                    />
                    <span>{derived.probeBadge.text}</span>
                  </div>

                  {/* 摄像机形态切换微控 */}
                  <div className="flex items-center rounded-xl border border-black/[0.06] bg-white/95 p-0.5 shadow-xs backdrop-blur-md dark:border-white/[0.08] dark:bg-black/50">
                    {(['bullet', 'dome', 'ptz'] as const).map((type) => (
                      <button
                        key={type}
                        type="button"
                        onClick={() => handleModelSelect(type)}
                        aria-pressed={modelType === type}
                        className={`min-h-7 rounded-lg px-2.5 text-[11px] font-medium transition-all focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
                          modelType === type
                            ? 'bg-slate-900 text-white shadow-xs dark:bg-slate-100 dark:text-slate-900'
                            : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                        }`}
                      >
                        {getCameraTypeLabel(type, t)}
                      </button>
                    ))}
                  </div>
                </div>

                {/* 居中硬件矢量展示台 */}
                <div className="relative my-4 flex h-36 w-full items-center justify-center sm:h-44">
                  <CameraIllustration
                    type={modelType}
                    status={derived.operationalStatus}
                    aiActive={derived.aiRuntime.isActive}
                    camera={currentCamera ?? undefined}
                    className="camera-illustration h-full max-h-[150px] w-auto max-w-[260px]"
                    ariaLabel={t('tile.illustrationLabel', {
                      type: typeLabel,
                      defaultValue: `${typeLabel} illustration`,
                    })}
                  />
                </div>
              </div>

              {/* ── 4. AI 管线与算法实例卡 ── */}
              <div className="overflow-hidden rounded-2xl border border-[var(--border)] bg-white p-4.5 dark:bg-[var(--bg-surface-solid)]">
                <div className="flex items-center justify-between gap-2 border-b border-[var(--border)]/60 pb-3">
                  <div className="flex items-center gap-2">
                    <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-[var(--accent-soft)] text-[var(--accent)]">
                      <Cpu className="h-4 w-4" />
                    </div>
                    <div>
                      <h3 className="text-sm font-semibold text-[var(--text-primary)]">
                        {t('drawer.aiPipeline', { defaultValue: 'AI pipeline' })}
                      </h3>
                    </div>
                  </div>

                  <span
                    className={`inline-flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-xs font-semibold ${PIPELINE_STATUS_STYLES[derived.aiRuntime.status]}`}
                  >
                    <span
                      className={`h-1.5 w-1.5 rounded-full bg-current ${derived.aiRuntime.status === 'active' ? 'animate-pulse' : ''}`}
                    />
                    <span>{pipelineStatusLabel}</span>
                  </span>
                </div>

                <div className="mt-3.5 grid grid-cols-2 gap-3 text-xs sm:grid-cols-3">
                  <div className="rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/30 p-2.5">
                    <span className="block text-[11px] text-[var(--text-muted)]">
                      {t('drawer.pipelineStatus', { defaultValue: 'Runtime status' })}
                    </span>
                    <span className="mt-0.5 block font-semibold text-[var(--text-primary)]">
                      {pipelineStatusLabel}
                    </span>
                  </div>
                  <div className="rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/30 p-2.5">
                    <span className="block text-[11px] text-[var(--text-muted)]">
                      {t('drawer.algorithmCount', { defaultValue: 'Active algorithms' })}
                    </span>
                    <span className="font-data mt-0.5 block font-semibold text-[var(--text-primary)] tabular-nums">
                      {derived.activeAlgorithmIds.length}
                    </span>
                  </div>
                  <div className="rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/30 p-2.5">
                    <span className="block text-[11px] text-[var(--text-muted)]">
                      {t('drawer.analysisStream', { defaultValue: 'Analysis stream' })}
                    </span>
                    <span className="mt-0.5 block truncate font-semibold text-[var(--text-primary)]">
                      {streamModeLabel}
                    </span>
                  </div>
                </div>

                {currentTask?.statusMessage && derived.aiRuntime.status !== 'active' && (
                  <p className="mt-3 rounded-xl border border-amber-500/30 bg-amber-500/10 p-2.5 text-xs leading-relaxed text-amber-700 dark:text-amber-300">
                    {currentTask.statusMessage}
                  </p>
                )}

                <div className="mt-3.5">
                  <div className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-[var(--text-secondary)]">
                    <Layers3 className="h-3.5 w-3.5 opacity-70" />
                    <span>
                      {t('drawer.activeAlgorithms', { defaultValue: 'Active AI algorithms' })}
                    </span>
                  </div>
                  {derived.activeAlgorithmIds.length > 0 ? (
                    <div className="divide-y divide-[var(--border)]/60 rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/20">
                      {derived.activeAlgorithmIds.map((algorithmId, index) => {
                        const instance = activeInstances[index]
                        return (
                          <div
                            key={algorithmId}
                            className="flex items-center justify-between gap-4 px-3 py-2 text-xs"
                          >
                            <span className="font-data min-w-0 truncate font-semibold text-[var(--text-primary)]">
                              {algorithmId}
                            </span>
                            <span className="font-data shrink-0 text-[var(--text-muted)] tabular-nums">
                              {instance?.analysisFps || task?.analysisFps
                                ? `${instance?.analysisFps || task?.analysisFps} FPS`
                                : t('tile.unknownValue', { defaultValue: '—' })}
                            </span>
                          </div>
                        )
                      })}
                    </div>
                  ) : (
                    <p className="rounded-xl border border-dashed border-[var(--border)] p-3 text-center text-xs text-[var(--text-muted)]">
                      {t('drawer.noActiveAlgorithms', { defaultValue: 'No active algorithms' })}
                    </p>
                  )}
                </div>
              </div>

              {/* ── 5. 流媒体传输配置卡 ── */}
              <div className="overflow-hidden rounded-2xl border border-[var(--border)] bg-white p-4.5 dark:bg-[var(--bg-surface-solid)]">
                <div className="flex items-center gap-2 border-b border-[var(--border)]/60 pb-3">
                  <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-[var(--accent-soft)] text-[var(--accent)]">
                    <Video className="h-4 w-4" />
                  </div>
                  <h3 className="text-sm font-semibold text-[var(--text-primary)]">
                    {t('drawer.streamConfiguration', { defaultValue: 'Stream configuration' })}
                  </h3>
                </div>

                <div className="mt-3.5 space-y-2.5">
                  {/* 主码流地址 */}
                  <div className="flex items-center justify-between gap-2 rounded-xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/30 p-2.5 text-xs">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-1.5">
                        <span className="font-semibold text-[var(--text-primary)]">
                          {t('tile.mainStream', { defaultValue: '主码流' })}
                        </span>
                        <span className="py-0.2 rounded-sm bg-[var(--accent-soft)] px-1 text-[10px] font-semibold text-[var(--accent)]">
                          MAIN
                        </span>
                      </div>
                      <span className="font-data mt-1 block text-[11px] break-all text-[var(--text-muted)]">
                        {currentCamera.rtspUrl || t('tile.unknownValue', { defaultValue: '—' })}
                      </span>
                    </div>
                    <button
                      type="button"
                      onClick={() => void handleCopy('main', currentCamera.rtspUrl)}
                      aria-label={t('tile.copyMainStream', {
                        defaultValue: '复制主码流 RTSP 地址',
                      })}
                      title={t('tile.copyMainStream', { defaultValue: '复制主码流 RTSP 地址' })}
                      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-[var(--border)]/60 bg-white text-[var(--text-muted)] shadow-xs transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none dark:bg-[var(--bg-surface-solid)]"
                    >
                      {copiedKey === 'main' ? (
                        <Check className="h-3.5 w-3.5 text-emerald-500" />
                      ) : (
                        <Copy className="h-3.5 w-3.5" />
                      )}
                    </button>
                  </div>

                  {/* 子码流地址 */}
                  <div className="flex items-center justify-between gap-2 rounded-xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/30 p-2.5 text-xs">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-1.5">
                        <span className="font-semibold text-[var(--text-primary)]">
                          {t('tile.subStream', { defaultValue: '子码流' })}
                        </span>
                        <span className="py-0.2 rounded-sm bg-[var(--bg-secondary)] px-1 text-[10px] font-semibold text-[var(--text-muted)]">
                          SUB
                        </span>
                      </div>
                      <span className="font-data mt-1 block text-[11px] break-all text-[var(--text-muted)]">
                        {currentCamera.subRtspUrl ||
                          t('drawer.noSubStreamHint', { defaultValue: '未配置子码流' })}
                      </span>
                    </div>
                    {currentCamera.subRtspUrl && (
                      <button
                        type="button"
                        onClick={() => void handleCopy('sub', currentCamera.subRtspUrl)}
                        aria-label={t('tile.copySubStream', {
                          defaultValue: '复制子码流 RTSP 地址',
                        })}
                        title={t('tile.copySubStream', { defaultValue: '复制子码流 RTSP 地址' })}
                        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-[var(--border)]/60 bg-white text-[var(--text-muted)] shadow-xs transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none dark:bg-[var(--bg-surface-solid)]"
                      >
                        {copiedKey === 'sub' ? (
                          <Check className="h-3.5 w-3.5 text-emerald-500" />
                        ) : (
                          <Copy className="h-3.5 w-3.5" />
                        )}
                      </button>
                    )}
                  </div>
                </div>

                {/* 遥测仪器指标矩阵 */}
                <div className="mt-3.5 grid grid-cols-2 gap-2.5 text-xs sm:grid-cols-3">
                  <div className="rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/25 p-2.5">
                    <span className="block text-[11px] text-[var(--text-muted)]">
                      {t('drawer.resolution', { defaultValue: 'Resolution' })}
                    </span>
                    <span className="font-data mt-0.5 flex items-center gap-1.5 font-semibold text-[var(--text-primary)] tabular-nums">
                      <Maximize2 className="h-3 w-3 text-[var(--text-muted)]" />
                      {currentCamera.lastWidth && currentCamera.lastHeight
                        ? `${currentCamera.lastWidth}×${currentCamera.lastHeight}`
                        : t('tile.unknownValue', { defaultValue: '—' })}
                    </span>
                  </div>
                  <div className="rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/25 p-2.5">
                    <span className="block text-[11px] text-[var(--text-muted)]">
                      {t('drawer.framerate', { defaultValue: '流帧率 (FPS)' })}
                    </span>
                    <span className="font-data mt-0.5 block font-semibold text-[var(--text-primary)] tabular-nums">
                      {currentCamera.lastFps
                        ? `${currentCamera.lastFps.toFixed(0)} FPS`
                        : t('tile.unknownValue', { defaultValue: '—' })}
                    </span>
                  </div>
                  <div className="rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/25 p-2.5">
                    <span className="block text-[11px] text-[var(--text-muted)]">
                      {t('drawer.codec', { defaultValue: 'Codec' })}
                    </span>
                    <span className="font-data mt-0.5 block font-semibold text-[var(--text-primary)] uppercase">
                      {currentCamera.lastCodec || t('tile.unknownValue', { defaultValue: 'hevc' })}
                    </span>
                  </div>
                  <div className="rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/25 p-2.5">
                    <span className="block text-[11px] text-[var(--text-muted)]">
                      {t('drawer.protocol', { defaultValue: '传输协议' })}
                    </span>
                    <span className="font-data mt-0.5 block font-semibold text-[var(--text-primary)] uppercase">
                      {currentCamera.protocol === 'gb28181' ? 'GB/T 28181' : 'RTSP / RTP'}
                    </span>
                  </div>
                  <div className="rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/25 p-2.5">
                    <span className="block text-[11px] text-[var(--text-muted)]">
                      {t('drawer.analysisStream', { defaultValue: 'Analysis stream' })}
                    </span>
                    <span className="mt-0.5 flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
                      <Split className="h-3 w-3 text-[var(--text-muted)]" />
                      {streamModeLabel}
                    </span>
                  </div>
                  <div className="rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/25 p-2.5">
                    <span className="block text-[11px] text-[var(--text-muted)]">
                      {t('drawer.createdAt', { defaultValue: '录入时间' })}
                    </span>
                    <span className="font-data mt-0.5 block truncate text-[var(--text-secondary)] tabular-nums">
                      {currentCamera.createdAt
                        ? formatTimestamp(currentCamera.createdAt)
                        : t('tile.unknownValue', { defaultValue: '—' })}
                    </span>
                  </div>
                </div>

                {currentCamera.remark && (
                  <div className="mt-3.5 flex items-start gap-2 rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/20 p-2.5 text-xs">
                    <FileText className="mt-0.5 h-3.5 w-3.5 shrink-0 text-[var(--text-muted)]" />
                    <span className="leading-relaxed text-[var(--text-secondary)]">
                      {currentCamera.remark}
                    </span>
                  </div>
                )}
              </div>

              {/* ── 6. 活动与保活时钟卡 ── */}
              <div className="overflow-hidden rounded-2xl border border-[var(--border)] bg-white p-4.5 dark:bg-[var(--bg-surface-solid)]">
                <div className="flex items-center gap-2 border-b border-[var(--border)]/60 pb-3">
                  <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-[var(--accent-soft)] text-[var(--accent)]">
                    <Clock3 className="h-4 w-4" />
                  </div>
                  <h3 className="text-sm font-semibold text-[var(--text-primary)]">
                    {t('drawer.activity', { defaultValue: 'Activity' })}
                  </h3>
                </div>

                <div className="mt-3.5 grid grid-cols-2 gap-3 text-xs">
                  <div className="rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/25 p-2.5">
                    <span className="block text-[11px] text-[var(--text-muted)]">
                      {t('drawer.lastActivity', { defaultValue: 'Last activity' })}
                    </span>
                    <span className="mt-0.5 block font-semibold text-[var(--text-primary)]">
                      {lastActivityLabel}
                    </span>
                  </div>
                  <div className="rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/25 p-2.5">
                    <span className="block text-[11px] text-[var(--text-muted)]">
                      {t('drawer.lastActivityAt', { defaultValue: 'Recorded at' })}
                    </span>
                    <span className="font-data mt-0.5 block text-[var(--text-secondary)] tabular-nums">
                      {lastActivityTimestamp}
                    </span>
                  </div>
                </div>
              </div>
            </div>
          </motion.aside>
        </div>
      )}
    </AnimatePresence>
  )
}
