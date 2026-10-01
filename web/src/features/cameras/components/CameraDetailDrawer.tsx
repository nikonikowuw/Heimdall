import React, { useMemo, useState } from 'react'
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
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Drawer } from '@/components/ui/Drawer'
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
  saveCameraModelType,
  useCameraModelType,
} from './illustrations/cameraModelType'
import type { CameraModelType, CameraOperationalStatus } from './illustrations/types'
import { RecordingConfigCard } from './RecordingConfigCard'

export interface CameraDetailDrawerProps {
  camera: Camera | null
  task?: CameraTaskRuntimeView | null
  onClose: () => void
  onManualProbe?: (camera: Camera) => void
  onEdit?: (camera: Camera) => void
  onDelete?: (camera: Camera) => void
  onModelTypeChange?: (cameraId: string, type: CameraModelType) => void
  /** 录像配置保存成功后回传最新摄像头对象，供列表与详情快照同步 */
  onRecordingConfigSaved?: (camera: Camera) => void
  isProbing?: boolean
  probeFeedback?: 'success' | 'failed'
}

const PIPELINE_STATUS_STYLES = {
  active: 'text-status-info bg-status-info/10 border-status-info/20',
  starting: 'text-status-warning bg-status-warning/10 border-status-warning/20',
  degraded: 'text-status-warning bg-status-warning/10 border-status-warning/20',
  error:
    'text-[var(--status-danger)] bg-[var(--status-danger-soft)] border-[var(--status-danger-border)]',
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
  online: 'text-status-success bg-status-success/10 border-status-success/20',
  warning: 'text-status-warning bg-status-warning/10 border-status-warning/20',
  offline:
    'text-[var(--status-neutral)] bg-[var(--status-neutral-soft)] border-[var(--status-neutral-border)]',
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
  onRecordingConfigSaved,
  isProbing = false,
  probeFeedback,
}: CameraDetailDrawerProps): React.ReactElement | null {
  const { t, i18n } = useTranslation('camera')
  const { t: tc } = useTranslation('common')
  const [copiedKey, setCopiedKey] = useState<string | null>(null)
  const [activeCamera, setActiveCamera] = useState<Camera | null>(camera)
  const [activeTask, setActiveTask] = useState<CameraTaskRuntimeView | null | undefined>(task)

  const isOpen = Boolean(camera)

  // 切换摄像头或任务数据更新时同步展示数据：用渲染期状态调整而非 effect，
  // 否则会先渲染一帧上一路摄像头的名称再被覆盖。
  //
  // 以 (camera, task) 的整体快照做标记而非只比 cameraId：task 的引用由父层
  // taskMap 重建，刷新后会换新对象，仅比 id 会漏掉这类更新。
  const [syncedView, setSyncedView] = useState<{
    camera: Camera | null
    task: CameraDetailDrawerProps['task']
  }>({
    camera,
    task,
  })
  if (camera !== syncedView.camera || task !== syncedView.task) {
    setSyncedView({ camera, task })
    if (camera) {
      setActiveCamera(camera)
      setActiveTask(task)
      setCopiedKey(null)
    }
  }

  const currentCamera = camera ?? activeCamera
  const currentTask = camera ? task : (activeTask ?? task)
  // 形态配置通过 useSyncExternalStore 响应式订阅外部存储与系统事件，
  // 杜绝渲染期读取 localStorage 的纯度违规。
  const modelType = useCameraModelType(currentCamera)

  const handleModelSelect = (nextType: CameraModelType) => {
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
  const closeLabel = t('drawer.close', { defaultValue: '关闭 (Esc)' })
  const drawerDescription = currentCamera && (
    <div className="flex items-center gap-1.5 text-xs text-[var(--text-muted)]">
      <span>{t('drawer.title', { defaultValue: '设备详情' })}</span>
      {currentCamera.remark && (
        <>
          <span>·</span>
          <span className="flex items-center gap-1 truncate text-[var(--text-secondary)]">
            <MapPin className="h-3 w-3 shrink-0 opacity-70" aria-hidden="true" />
            <span className="truncate">{currentCamera.remark}</span>
          </span>
        </>
      )}
    </div>
  )
  const drawerMetadata = currentCamera && (
    <>
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
          <Check className="text-status-success h-2.5 w-2.5" />
        ) : (
          <Copy className="h-2.5 w-2.5 opacity-60 group-hover/id:opacity-100" />
        )}
      </button>
      <span className="rounded-md border border-[var(--border)] bg-[var(--bg-secondary)] px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[var(--text-muted)]">
        {currentCamera.protocol.toUpperCase()}
      </span>
    </>
  )
  const drawerToolbar = currentCamera && (
    <div className="flex w-full flex-wrap items-center justify-between gap-2">
      <div className="flex items-center gap-2">
        {onEdit && (
          <button
            type="button"
            onClick={() => onEdit(currentCamera)}
            className="flex min-h-8.5 items-center gap-1.5 rounded-xl bg-[var(--surface-inverse)] px-3.5 text-xs font-semibold text-[var(--on-inverse)] shadow-xs transition-colors hover:bg-[var(--surface-inverse-hover)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none dark:hover:bg-white"
          >
            <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
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
            <RefreshCw
              className={`h-3.5 w-3.5 ${isProbing ? 'animate-spin' : ''}`}
              aria-hidden="true"
            />
            <span>{getProbeButtonText(isProbing, probeFeedback, t)}</span>
          </button>
        )}
      </div>
      {onDelete && (
        <button
          type="button"
          onClick={() => onDelete(currentCamera)}
          className="flex min-h-8.5 items-center gap-1.5 rounded-xl px-2.5 text-xs font-medium text-[var(--status-danger)] transition-colors hover:bg-[var(--status-danger-soft)] focus-visible:ring-2 focus-visible:ring-[var(--status-danger)]/40 focus-visible:outline-none"
        >
          <Trash2 className="h-3.5 w-3.5 opacity-80" aria-hidden="true" />
          <span>{tc('actions.delete')}</span>
        </button>
      )}
    </div>
  )

  return (
    <Drawer
      isOpen={isOpen && Boolean(currentCamera && derived)}
      onClose={onClose}
      closeLabel={closeLabel}
      closeTitle={closeLabel}
      title={currentCamera?.name || currentCamera?.cameraId || ''}
      titleTooltip={currentCamera?.name || currentCamera?.cameraId || ''}
      description={drawerDescription}
      icon={<Video className="h-5 w-5" aria-hidden="true" />}
      metadata={drawerMetadata}
      toolbar={drawerToolbar}
      size="medium"
      bodyClassName="space-y-5"
      onExitComplete={() => {
        if (!camera) {
          setActiveCamera(null)
          setActiveTask(null)
        }
      }}
    >
      {currentCamera && derived && (
        <>
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
              <div className="flex items-center rounded-xl border border-black/[0.06] bg-white/95 p-0.5 shadow-xs dark:border-white/[0.08] dark:bg-black/50">
                {(['bullet', 'dome', 'ptz'] as const).map((type) => (
                  <button
                    key={type}
                    type="button"
                    onClick={() => handleModelSelect(type)}
                    aria-pressed={modelType === type}
                    className={`min-h-7 rounded-lg px-2.5 text-[11px] font-medium transition-all focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
                      modelType === type
                        ? 'bg-[var(--surface-inverse)] text-[var(--on-inverse)] shadow-xs'
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
                className="h-full max-h-[150px] w-auto max-w-[260px]"
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
              <p className="border-status-warning/30 bg-status-warning/10 text-status-warning mt-3 rounded-xl border p-2.5 text-xs leading-relaxed">
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
                    <span className="rounded-sm bg-[var(--accent-soft)] px-1 text-[10px] font-semibold text-[var(--accent)]">
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
                    <Check className="text-status-success h-3.5 w-3.5" />
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
                    <span className="rounded-sm bg-[var(--bg-secondary)] px-1 text-[10px] font-semibold text-[var(--text-muted)]">
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
                      <Check className="text-status-success h-3.5 w-3.5" />
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

          {/* ── 5.5 事件录像配置卡：开关即时保存，参数走显式保存 ── */}
          <RecordingConfigCard
            camera={currentCamera}
            onSaved={(updated) => {
              setActiveCamera(updated)
              onRecordingConfigSaved?.(updated)
            }}
          />

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
        </>
      )}
    </Drawer>
  )
}
