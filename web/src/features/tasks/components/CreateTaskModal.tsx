import React, { useEffect, useMemo, useState } from 'react'
import {
  AlertCircle,
  Check,
  Cpu,
  Loader2,
  Plus,
  Search,
  Sliders,
  Sparkles,
  Video,
  X,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { algorithmApi, isConfigConflictError, taskApi } from '@/lib/api'
import type { AlgorithmItem, Camera, TaskConfigDto } from '@/types'
import { extractTargetClasses } from '../algoMetadata'
import {
  buildQuickCreatePayload,
  pickActiveVersion,
  pickRecommendedAlgorithmId,
  splitCamerasByTask,
} from '../taskDraft'
import { getLocalizedClassName } from './rulesStudioTypes'

export interface CreateTaskModalProps {
  isOpen: boolean
  cameras: Camera[]
  existingCameraIdsWithTasks: ReadonlySet<string>
  preselectedCameraId?: string | null
  onClose: () => void
  onSuccess: (camera: Camera, task: TaskConfigDto) => void
  onGoToCameras?: () => void
  onGoToAlgorithms?: () => void
}

function formatDefaultTaskName(cam?: Camera): string {
  if (!cam) return ''
  return `Task-${cam.name || cam.cameraId}`
}

/**
 * 快速创建布防任务。
 *
 * 创建阶段只确定「通道 + 名称 + 初始算法 + 是否立即布防」：没有画面可参照时，
 * 置信度、目标类别、推理帧率与空间防区都无从判断，一律留到布防工作台的
 * 动态子码流画布上微调。已建立任务的通道不会出现在候选中（一设备一任务）。
 */
export function CreateTaskModal({
  isOpen,
  cameras,
  existingCameraIdsWithTasks,
  preselectedCameraId,
  onClose,
  onSuccess,
  onGoToCameras,
  onGoToAlgorithms,
}: CreateTaskModalProps): React.ReactElement | null {
  const { t, i18n } = useTranslation('task')
  const { t: tc } = useTranslation('common')

  const [selectedCameraId, setSelectedCameraId] = useState<string>('')
  const [taskName, setTaskName] = useState<string>('')
  /** 用户是否手工改过任务名：改过之后切换通道不再覆盖 */
  const [isNameEdited, setIsNameEdited] = useState<boolean>(false)
  const [channelQuery, setChannelQuery] = useState<string>('')
  const [desiredEnabled, setDesiredEnabled] = useState<boolean>(false)

  const [availableAlgorithms, setAvailableAlgorithms] = useState<AlgorithmItem[]>([])
  const [selectedAlgorithmId, setSelectedAlgorithmId] = useState<string>('')

  const [isSubmitting, setIsSubmitting] = useState<boolean>(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)

  const { creatable, configuredCount } = useMemo(
    () => splitCamerasByTask(cameras, existingCameraIdsWithTasks),
    [cameras, existingCameraIdsWithTasks],
  )

  const visibleCameras = useMemo(() => {
    const keyword = channelQuery.trim().toLowerCase()
    if (!keyword) return creatable
    return creatable.filter(
      (cam) =>
        cam.name.toLowerCase().includes(keyword) || cam.cameraId.toLowerCase().includes(keyword),
    )
  }, [creatable, channelQuery])

  const selectedAlgo = availableAlgorithms.find((item) => item.algorithmId === selectedAlgorithmId)
  const activeVersion = useMemo(() => pickActiveVersion(selectedAlgo), [selectedAlgo])
  const recommendedAlgorithmId = useMemo(
    () => pickRecommendedAlgorithmId(availableAlgorithms),
    [availableAlgorithms],
  )
  const selectedClasses = useMemo(() => extractTargetClasses(activeVersion), [activeVersion])

  function handleSelectCamera(cam: Camera): void {
    setSelectedCameraId(cam.cameraId)
    if (!isNameEdited) {
      setTaskName(formatDefaultTaskName(cam))
    }
  }

  function handleGoToCameras(): void {
    onClose()
    onGoToCameras?.()
  }

  function handleGoToAlgorithms(): void {
    onClose()
    onGoToAlgorithms?.()
  }

  // 打开弹窗时加载可用算法，并预选系统推荐项（与后端 resolve_algorithm_id 同口径）
  useEffect(() => {
    if (!isOpen) return
    let active = true
    algorithmApi
      .list({ page: 1, pageSize: 100 })
      .then((result) => {
        if (!active) return
        setAvailableAlgorithms(result.items)
        setSelectedAlgorithmId(pickRecommendedAlgorithmId(result.items))
      })
      .catch(() => {
        if (!active) return
        setAvailableAlgorithms([])
        setSelectedAlgorithmId('')
      })
    return () => {
      active = false
    }
  }, [isOpen])

  // 打开弹窗时初始化：候选通道中优先选中预设通道，其次首个未配置通道
  useEffect(() => {
    if (!isOpen) return
    setErrorMsg(null)
    setChannelQuery('')
    setIsSubmitting(false)
    setDesiredEnabled(false)

    const preselected = preselectedCameraId
      ? creatable.find((c) => c.cameraId === preselectedCameraId)
      : undefined
    const defaultCam = preselected ?? creatable[0]
    setSelectedCameraId(defaultCam?.cameraId ?? '')
    setTaskName(formatDefaultTaskName(defaultCam))
    setIsNameEdited(false)
  }, [isOpen, creatable, preselectedCameraId])

  useDismissStack(isOpen, onClose, { disabled: isSubmitting })

  if (!isOpen) return null

  async function handleSubmit(e: React.FormEvent): Promise<void> {
    e.preventDefault()

    const cam = creatable.find((c) => c.cameraId === selectedCameraId)
    if (!cam) {
      setErrorMsg(t('validation.cameraNotFound', { defaultValue: '所选摄像头不存在' }))
      return
    }
    if (!taskName.trim()) {
      setErrorMsg(t('validation.nameRequired', { defaultValue: '任务名称不能为空' }))
      return
    }
    if (!selectedAlgorithmId) {
      setErrorMsg(
        t('noAlgorithmToBind', {
          defaultValue: '尚无可用算法包，请先在算法管理中上传并激活算法。',
        }),
      )
      return
    }

    setIsSubmitting(true)
    setErrorMsg(null)

    try {
      const payload = buildQuickCreatePayload({
        cameraId: cam.cameraId,
        name: taskName,
        algorithmId: selectedAlgorithmId,
        algorithmVersion: activeVersion,
        desiredEnabled,
      })
      const created = await taskApi.updateTask(cam.cameraId, payload)
      onSuccess(cam, created)
      onClose()
    } catch (err) {
      if (isConfigConflictError(err)) {
        // 该通道在本次操作期间已被其他会话建立任务：提示重新选择，避免覆盖对方配置
        setErrorMsg(
          t('errors.cameraTaskConflict', {
            defaultValue: '该通道已被其他会话创建了 AI 任务，请刷新列表后重试',
          }),
        )
      } else {
        const msg =
          err instanceof Error
            ? err.message
            : t('errors.createFailed', { defaultValue: '创建任务失败，请稍后重试' })
        setErrorMsg(msg)
      }
    } finally {
      setIsSubmitting(false)
    }
  }

  function renderBody(): React.ReactElement {
    if (cameras.length === 0) {
      return (
        <div className="flex flex-col items-center justify-center p-6 text-center text-xs">
          <Video className="mb-2 h-8 w-8 text-[var(--text-muted)] opacity-50" />
          <p className="font-semibold text-[var(--text-primary)]">
            {t('noCamerasAvailable', {
              defaultValue: '系统中暂无任何摄像头设备，请先接入摄像机',
            })}
          </p>
          <p className="mt-1 text-[var(--text-muted)]">
            {t('noCamerasHint', { defaultValue: 'AI 任务需要绑定在有效的视频流通道上运行。' })}
          </p>
          <button
            type="button"
            onClick={handleGoToCameras}
            className="mt-4 flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{t('goToCameras', { defaultValue: '前往设备管理' })}</span>
          </button>
        </div>
      )
    }

    if (creatable.length === 0) {
      return (
        <div className="flex flex-col items-center justify-center p-6 text-center text-xs">
          <Video className="mb-2 h-8 w-8 text-[var(--text-muted)] opacity-50" />
          <p className="font-semibold text-[var(--text-primary)]">
            {t('allCamerasAssigned', { defaultValue: '所有已接入的摄像头均已建立布防任务' })}
          </p>
          <p className="mt-1 text-[var(--text-muted)]">
            {t('oneTaskPerChannelHint', {
              defaultValue: '每个通道同一时间只承载一个任务；调整算法与防区请进入布防工作台。',
            })}
          </p>
          <button
            type="button"
            onClick={onClose}
            className="mt-4 flex items-center gap-1.5 rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-semibold text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
          >
            <span>{t('viewConfiguredTasks', { defaultValue: '查看任务列表' })}</span>
          </button>
        </div>
      )
    }

    return (
      <form onSubmit={handleSubmit} className="flex min-h-0 flex-1 flex-col">
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-5 text-xs">
          {/* 通道选择 */}
          <fieldset>
            <legend className="mb-1.5 block font-semibold text-[var(--text-primary)]">
              {t('selectChannel', { defaultValue: '选择摄像头通道' })}
              <span className="ml-1 text-[var(--destructive)]">*</span>
            </legend>

            <div className="relative mb-2">
              <Search
                className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)]"
                aria-hidden="true"
              />
              <input
                type="search"
                value={channelQuery}
                onChange={(e) => setChannelQuery(e.target.value)}
                placeholder={t('channelSearchPlaceholder', {
                  defaultValue: '搜索通道名称 / ID',
                })}
                aria-label={t('channelSearchPlaceholder', {
                  defaultValue: '搜索通道名称 / ID',
                })}
                disabled={isSubmitting}
                className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] py-2 pr-3 pl-8 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]"
              />
            </div>

            {visibleCameras.length === 0 ? (
              <p className="rounded-xl border border-dashed border-[var(--border)] bg-[var(--bg-surface)] p-3 text-[11px] text-[var(--text-muted)]">
                {t('noMatchingChannels', { defaultValue: '没有匹配的通道' })}
              </p>
            ) : (
              <div className="grid max-h-56 grid-cols-1 gap-2 overflow-y-auto pr-0.5 sm:grid-cols-2">
                {visibleCameras.map((cam) => {
                  const isSelected = cam.cameraId === selectedCameraId
                  return (
                    <button
                      key={cam.cameraId}
                      type="button"
                      onClick={() => handleSelectCamera(cam)}
                      disabled={isSubmitting}
                      aria-pressed={isSelected}
                      className={`flex flex-col gap-1 rounded-xl border p-2.5 text-left transition-all focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
                        isSelected
                          ? 'border-[var(--border-strong)] bg-[var(--accent-soft)] shadow-xs'
                          : 'border-[var(--border)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]'
                      }`}
                    >
                      <span className="flex items-center justify-between gap-2">
                        <span className="flex min-w-0 items-center gap-1.5">
                          <Video
                            className={`h-3.5 w-3.5 shrink-0 ${
                              isSelected ? 'text-[var(--accent)]' : 'text-[var(--text-muted)]'
                            }`}
                          />
                          <span className="truncate font-semibold text-[var(--text-primary)]">
                            {cam.name || cam.cameraId}
                          </span>
                        </span>
                        {isSelected && (
                          <Check className="h-3.5 w-3.5 shrink-0 text-[var(--accent)]" />
                        )}
                      </span>
                      <span className="font-data flex items-center gap-1.5 text-[10px] text-[var(--text-muted)] tabular-nums">
                        <span>
                          {cam.lastWidth && cam.lastHeight
                            ? `${cam.lastWidth}×${cam.lastHeight}`
                            : '—'}
                        </span>
                        <span>·</span>
                        <span className="text-[var(--accent)]">
                          {cam.lastCodec?.toUpperCase() || 'H264'}
                        </span>
                      </span>
                    </button>
                  )
                })}
              </div>
            )}

            {configuredCount > 0 && (
              <button
                type="button"
                onClick={onClose}
                className="mt-2 text-[11px] text-[var(--text-muted)] transition-colors hover:text-[var(--accent)]"
              >
                {t('otherConfiguredChannels', {
                  count: configuredCount,
                  defaultValue: '另有 {{count}} 路通道已建立任务，前往任务列表调整',
                })}
              </button>
            )}
          </fieldset>

          {/* 任务名称 */}
          <div>
            <label
              htmlFor="create-task-name"
              className="mb-1.5 block font-semibold text-[var(--text-primary)]"
            >
              {t('taskName', { defaultValue: '任务名称' })}
              <span className="ml-1 text-[var(--destructive)]">*</span>
            </label>
            <input
              id="create-task-name"
              type="text"
              value={taskName}
              onChange={(e) => {
                setTaskName(e.target.value)
                setIsNameEdited(true)
              }}
              placeholder={t('taskNamePlaceholder', {
                defaultValue: '例如：周界防范 - 库房正门',
              })}
              disabled={isSubmitting}
              className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]"
            />
          </div>

          {/* 初始算法 */}
          <fieldset>
            <legend className="mb-1.5 flex w-full items-center justify-between font-semibold text-[var(--text-primary)]">
              <span>
                {t('algorithm', { defaultValue: '分析算法' })}
                <span className="ml-1 text-[var(--destructive)]">*</span>
              </span>
              {selectedAlgo && activeVersion && (
                <span className="font-data text-[10px] font-normal text-[var(--text-muted)] tabular-nums">
                  {activeVersion.platformId} · v{activeVersion.version}
                </span>
              )}
            </legend>

            {availableAlgorithms.length === 0 ? (
              <div className="rounded-xl border border-dashed border-[var(--border)] bg-[var(--bg-surface)] p-3 text-[11px]">
                <p className="text-[var(--text-muted)]">
                  {t('noAlgorithmToBind', {
                    defaultValue: '尚无可用算法包，请先在算法管理中上传并激活算法。',
                  })}
                </p>
                <button
                  type="button"
                  onClick={handleGoToAlgorithms}
                  className="mt-2 flex items-center gap-1.5 text-[var(--accent)] hover:underline"
                >
                  <Cpu className="h-3.5 w-3.5" />
                  <span>{t('goToAlgorithms', { defaultValue: '前往算法管理' })}</span>
                </button>
              </div>
            ) : (
              <div className="grid max-h-48 grid-cols-1 gap-2 overflow-y-auto pr-0.5 sm:grid-cols-2">
                {availableAlgorithms.map((algo) => {
                  const isSelected = algo.algorithmId === selectedAlgorithmId
                  const version = pickActiveVersion(algo)
                  const classCount = extractTargetClasses(version).length
                  const isRecommended = algo.algorithmId === recommendedAlgorithmId
                  return (
                    <button
                      key={algo.algorithmId}
                      type="button"
                      onClick={() => setSelectedAlgorithmId(algo.algorithmId)}
                      disabled={isSubmitting}
                      aria-pressed={isSelected}
                      className={`flex flex-col gap-1 rounded-xl border p-2.5 text-left transition-all focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
                        isSelected
                          ? 'border-[var(--border-strong)] bg-[var(--accent-soft)] shadow-xs'
                          : 'border-[var(--border)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]'
                      }`}
                    >
                      <span className="flex items-center justify-between gap-2">
                        <span className="truncate font-semibold text-[var(--text-primary)]">
                          {algo.name}
                        </span>
                        {isSelected && (
                          <Check className="h-3.5 w-3.5 shrink-0 text-[var(--accent)]" />
                        )}
                      </span>
                      <span className="font-data flex items-center gap-1.5 text-[10px] text-[var(--text-muted)] tabular-nums">
                        <span className="truncate">{algo.algorithmId}</span>
                        {classCount > 0 && (
                          <>
                            <span>·</span>
                            <span>{classCount} 类</span>
                          </>
                        )}
                        {isRecommended && (
                          <>
                            <span>·</span>
                            <span className="inline-flex items-center gap-1 text-[var(--accent)]">
                              <Sparkles className="h-3 w-3" />
                              {t('algorithmRecommended', { defaultValue: '系统推荐' })}
                            </span>
                          </>
                        )}
                      </span>
                    </button>
                  )
                })}
              </div>
            )}

            {selectedClasses.length > 0 && (
              <p className="mt-1.5 flex flex-wrap gap-1 text-[10px] text-[var(--text-muted)]">
                {selectedClasses.slice(0, 8).map((cls) => (
                  <span
                    key={cls}
                    className="rounded-md border border-[var(--border)] px-1.5 py-0.5"
                  >
                    {getLocalizedClassName(cls, i18n.language)}
                  </span>
                ))}
              </p>
            )}
          </fieldset>

          {/* 立即布防 */}
          <div className="flex items-center justify-between gap-3 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3">
            <div className="min-w-0">
              <span className="font-semibold text-[var(--text-primary)]">
                {t('enableArmImmediately', { defaultValue: '创建后立即启动布防' })}
              </span>
              <p className="mt-0.5 text-[11px] text-[var(--text-muted)]">
                {desiredEnabled
                  ? t('enableArmImmediatelyDesc', {
                      defaultValue:
                        '开启后系统将启动该路摄像头的分析码流解码并在后台调度 NPU 规则判定。',
                    })
                  : t('armLaterHint', {
                      defaultValue:
                        '默认暂不布防：未划定防区时按全画幅判定，建议先在工作台画好防区再开闸。',
                    })}
              </p>
            </div>
            <button
              type="button"
              onClick={() => setDesiredEnabled((prev) => !prev)}
              disabled={isSubmitting}
              aria-pressed={desiredEnabled}
              aria-label={t('enableArmImmediately', { defaultValue: '创建后立即启动布防' })}
              className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full transition-colors focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none ${
                desiredEnabled
                  ? 'bg-[var(--accent)]'
                  : 'border border-[var(--border)] bg-[var(--bg-secondary)]'
              }`}
            >
              <span
                className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow-lg transition-transform ${
                  desiredEnabled ? 'translate-x-4' : 'translate-x-0.5'
                }`}
              />
            </button>
          </div>

          {errorMsg && (
            <div
              role="alert"
              className="flex items-center gap-2 rounded-lg border border-[var(--destructive)]/30 bg-[var(--destructive)]/10 p-3 text-xs text-[var(--destructive)]"
            >
              <AlertCircle className="h-4 w-4 shrink-0" />
              <span>{errorMsg}</span>
            </div>
          )}
        </div>

        {/* 底部操作栏 */}
        <div className="flex shrink-0 items-center justify-between gap-3 border-t border-[var(--border)] p-4">
          <button
            type="button"
            onClick={onClose}
            disabled={isSubmitting}
            className="rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-semibold text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
          >
            {tc('cancel')}
          </button>

          <button
            type="submit"
            disabled={isSubmitting || !selectedCameraId || !selectedAlgorithmId}
            className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95 disabled:opacity-50"
          >
            {isSubmitting ? (
              <>
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                <span>{t('creating', { defaultValue: '创建中...' })}</span>
              </>
            ) : (
              <>
                <Check className="h-3.5 w-3.5" />
                <span>{t('confirmAndDrawRules', { defaultValue: '创建并进入工作台' })}</span>
              </>
            )}
          </button>
        </div>
      </form>
    )
  }

  return (
    <div
      onClick={(e) => {
        if (e.target === e.currentTarget && !isSubmitting) {
          onClose()
        }
      }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-[var(--overlay-scrim)] p-4 backdrop-blur-xs"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="create-task-title"
        className="relative flex max-h-[90vh] w-full max-w-xl flex-col rounded-2xl border border-[var(--border-strong)] bg-[var(--bg-surface-solid)] shadow-2xl"
      >
        {/* 头部 */}
        <div className="shrink-0 rounded-t-2xl border-b border-[var(--border)] bg-[var(--bg-secondary)]/45 p-5 pb-4">
          <button
            type="button"
            onClick={onClose}
            disabled={isSubmitting}
            aria-label={tc('close')}
            className="absolute top-5 right-5 rounded-lg p-1 text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
          >
            <X className="h-4 w-4" />
          </button>

          <div className="flex items-center gap-3 pr-8">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-[var(--border)] bg-[var(--accent-soft)] text-[var(--accent)]">
              <Sliders className="h-5 w-5" />
            </div>
            <div>
              <h3 id="create-task-title" className="text-base font-bold text-[var(--text-primary)]">
                {t('createTaskTitle', { defaultValue: '创建 AI 分析与布防任务' })}
              </h3>
              <p className="text-xs text-[var(--text-muted)]">
                {t('quickCreateSubtitle', {
                  defaultValue:
                    '选择通道与算法即可建立任务，防区与识别参数稍后在布防工作台中微调。',
                })}
              </p>
            </div>
          </div>
        </div>

        {/* 正文 */}
        {renderBody()}
      </div>
    </div>
  )
}
