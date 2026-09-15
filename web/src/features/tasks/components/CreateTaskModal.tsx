import React, { useEffect, useMemo, useState } from 'react'
import {
  AlertCircle,
  ArrowLeft,
  ArrowRight,
  Check,
  CheckCircle2,
  Loader2,
  Plus,
  Sliders,
  Video,
  X,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { StreamModeSelector } from '@/components/StreamModeSelector'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { algorithmApi, taskApi } from '@/lib/api'
import type { AlgorithmItem, Camera, StreamMode, TaskConfigDto } from '@/types'
import { extractConfigProperties, extractTargetClasses } from '../algoMetadata'
import { getLocalizedClassName } from './rulesStudioTypes'

export interface CreateTaskModalProps {
  isOpen: boolean
  cameras: Camera[]
  existingCameraIdsWithTasks: Set<string>
  preselectedCameraId?: string | null
  onClose: () => void
  onSuccess: (camera: Camera, task: TaskConfigDto) => void
  onGoToCameras: () => void
}

const FPS_PRESETS = [5, 10, 15, 25] as const

/** 依据算法 configSchema 生成推荐初始参数；深度调参在工作台内完成 */
function buildDefaultParams(
  version: AlgorithmItem['versions'][number] | undefined,
): Record<string, unknown> {
  const properties = extractConfigProperties(version)
  const params: Record<string, unknown> = {}
  for (const [key, prop] of Object.entries(properties)) {
    if (prop.default !== undefined) {
      params[key] = prop.default
    } else if (prop.type === 'number' || prop.type === 'integer') {
      params[key] = prop.minimum ?? 0
    } else if (prop.type === 'boolean') {
      params[key] = false
    } else if (prop.type === 'string') {
      params[key] = (prop.enum as string[] | undefined)?.[0] ?? ''
    } else if (prop.type === 'array') {
      params[key] = []
    }
  }
  return params
}

export function CreateTaskModal({
  isOpen,
  cameras,
  existingCameraIdsWithTasks,
  preselectedCameraId,
  onClose,
  onSuccess,
  onGoToCameras,
}: CreateTaskModalProps): React.ReactElement | null {
  const { t, i18n } = useTranslation('task')
  const { t: tc } = useTranslation('common')

  const [step, setStep] = useState<1 | 2>(1)

  // 第 1 步：通道与任务身份
  const [selectedCameraId, setSelectedCameraId] = useState<string>('')
  const [taskName, setTaskName] = useState<string>('')
  const [streamMode, setStreamMode] = useState<StreamMode>('auto')
  const [desiredEnabled, setDesiredEnabled] = useState<boolean>(true)

  // 第 2 步：算法与推理预算
  const [availableAlgorithms, setAvailableAlgorithms] = useState<AlgorithmItem[]>([])
  const [selectedAlgorithmId, setSelectedAlgorithmId] = useState<string>('')
  const [analysisFps, setAnalysisFps] = useState<number>(10)
  const [confidence, setConfidence] = useState<number>(0.45)
  const [targetClasses, setTargetClasses] = useState<string[]>([])

  const [isSubmitting, setIsSubmitting] = useState<boolean>(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)

  const selectedCam = cameras.find((c) => c.cameraId === selectedCameraId)
  const selectedAlgo = availableAlgorithms.find((item) => item.algorithmId === selectedAlgorithmId)
  const activeVersion = useMemo(() => {
    if (!selectedAlgo) return undefined
    return (
      selectedAlgo.versions.find((v) => v.isActive) ||
      selectedAlgo.versions.find((v) => v.version === selectedAlgo.activeVersion) ||
      selectedAlgo.versions[0]
    )
  }, [selectedAlgo])

  const availableClasses = useMemo(() => extractTargetClasses(activeVersion), [activeVersion])
  const configuredClassCount = targetClasses.length

  // 打开弹窗时加载可用算法
  useEffect(() => {
    if (!isOpen) return
    let active = true
    algorithmApi
      .list({ page: 1, pageSize: 100 })
      .then((result) => {
        if (!active) return
        setAvailableAlgorithms(result.items)
        const defaultAlgorithm =
          result.items.find((item) => item.activeVersion.trim() !== '') ?? result.items[0]
        setSelectedAlgorithmId(defaultAlgorithm?.algorithmId ?? '')
      })
      .catch(() => {
        if (active) {
          setAvailableAlgorithms([])
          setSelectedAlgorithmId('')
        }
      })
    return () => {
      active = false
    }
  }, [isOpen])

  // 打开弹窗时初始化第 1 步
  useEffect(() => {
    if (!isOpen) return
    setErrorMsg(null)
    setStep(1)
    const preselected = preselectedCameraId
      ? cameras.find((c) => c.cameraId === preselectedCameraId)
      : undefined
    const defaultCam =
      preselected ?? cameras.find((c) => !existingCameraIdsWithTasks.has(c.cameraId)) ?? cameras[0]

    if (defaultCam) {
      setSelectedCameraId(defaultCam.cameraId)
      setTaskName(`Task-${defaultCam.name || defaultCam.cameraId}`)
      setStreamMode(defaultCam.streamMode || 'auto')
    } else {
      setSelectedCameraId('')
      setTaskName('')
      setStreamMode('auto')
    }
    setDesiredEnabled(true)
    setAnalysisFps(10)
    setConfidence(0.45)
  }, [isOpen, cameras, existingCameraIdsWithTasks, preselectedCameraId])

  // 切换算法时刷新目标类别与置信度默认值
  useEffect(() => {
    if (!activeVersion) {
      setTargetClasses([])
      return
    }
    const classes = extractTargetClasses(activeVersion)
    setTargetClasses(classes)
    const properties = extractConfigProperties(activeVersion)
    const confProp =
      properties.confidence_threshold ??
      properties.confidenceThreshold ??
      properties.detection_confidence_threshold
    const defaultConfidence = confProp?.default
    setConfidence(typeof defaultConfidence === 'number' ? defaultConfidence : 0.45)
  }, [activeVersion])

  useDismissStack(isOpen, onClose, { disabled: isSubmitting })

  if (!isOpen) return null

  const handleSelectCamera = (cam: Camera) => {
    setSelectedCameraId(cam.cameraId)
    setTaskName(`Task-${cam.name || cam.cameraId}`)
    setStreamMode(cam.streamMode || 'auto')
  }

  const handleGoToStepTwo = () => {
    if (!selectedCameraId) {
      setErrorMsg(t('selectChannelPlaceholder', { defaultValue: '请选择要分配任务的摄像头通道' }))
      return
    }
    if (!taskName.trim()) {
      setErrorMsg(t('validation.nameRequired', { defaultValue: '任务名称不能为空' }))
      return
    }
    setErrorMsg(null)
    setStep(2)
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    const cam = cameras.find((c) => c.cameraId === selectedCameraId)
    if (!cam) {
      setErrorMsg(t('validation.cameraNotFound', { defaultValue: '所选摄像头不存在' }))
      setStep(1)
      return
    }

    setIsSubmitting(true)
    setErrorMsg(null)

    try {
      const baseParams = buildDefaultParams(activeVersion)
      const algoParams: Record<string, unknown> = {
        ...baseParams,
        confidence_threshold: confidence,
        confidenceThreshold: confidence,
        target_classes: targetClasses,
        targetClasses: targetClasses,
      }

      const payload: TaskConfigDto = {
        cameraId: selectedCameraId,
        name: taskName.trim(),
        desiredEnabled,
        streamMode,
        rules: [],
        motionGate: {
          enabled: true,
          threshold: 25,
          contourArea: 100,
          keepaliveIntervalMs: 2000,
          motionHoldFrames: 10,
        },
        algorithmInstances: selectedAlgorithmId
          ? [
              {
                algorithmId: selectedAlgorithmId,
                analysisFps,
                algoParams,
                enabled: desiredEnabled,
              },
            ]
          : undefined,
      }

      const created = await taskApi.updateTask(selectedCameraId, payload)
      onSuccess({ ...cam, streamMode }, created)
      onClose()
    } catch (err) {
      const msg =
        err instanceof Error
          ? err.message
          : t('errors.createFailed', { defaultValue: '创建任务失败，请稍后重试' })
      setErrorMsg(msg)
    } finally {
      setIsSubmitting(false)
    }
  }

  return (
    <div
      onClick={(e) => {
        if (e.target === e.currentTarget && !isSubmitting) {
          onClose()
        }
      }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-[#030408]/80 p-4 backdrop-blur-xs"
    >
      <div className="relative flex max-h-[90vh] w-full max-w-3xl flex-col rounded-[10px] border border-t-2 border-[var(--border-strong)] border-t-[var(--accent)] bg-[var(--bg-surface-solid)] shadow-2xl">
        {/* 头部：标题 + 步骤指示 */}
        <div className="shrink-0 border-b border-[var(--border)] bg-[var(--bg-secondary)]/45 p-5 pb-4">
          <button
            type="button"
            onClick={onClose}
            disabled={isSubmitting}
            aria-label={tc('actions.cancel', { defaultValue: '取消' })}
            className="absolute top-5 right-5 rounded-lg p-1 text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
          >
            <X className="h-4 w-4" />
          </button>

          <div className="flex items-center gap-3 pr-8">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[8px] border border-[var(--accent)]/25 bg-[var(--accent-soft)] text-[var(--accent)]">
              <Sliders className="h-5 w-5" />
            </div>
            <div>
              <h3 className="text-base font-bold text-[var(--text-primary)]">
                {t('createTaskTitle', { defaultValue: '创建 AI 分析与布防任务' })}
              </h3>
              <p className="text-xs text-[var(--text-muted)]">
                {t('wizard.subtitle', {
                  defaultValue: '两步建立任务：绑定通道 → 选择算法，空间防区在布防工作台内绘制。',
                })}
              </p>
            </div>
          </div>

          {/* 步骤条 */}
          <div className="mt-4 flex items-center gap-2 md:hidden">
            {([1, 2] as const).map((index) => {
              const isDone = step > index
              const isCurrent = step === index
              const stepLabel =
                index === 1
                  ? t('wizard.stepChannel', { defaultValue: '选择通道' })
                  : t('wizard.stepAlgorithm', { defaultValue: '选择算法' })
              return (
                <React.Fragment key={index}>
                  {index === 2 && (
                    <span className="h-px flex-1 bg-[var(--border)]" aria-hidden="true" />
                  )}
                  <span
                    className={`flex shrink-0 items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-semibold transition-colors ${
                      isCurrent
                        ? 'border-[var(--accent)] bg-[var(--accent-soft)] text-[var(--accent)]'
                        : isDone
                          ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-500'
                          : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-muted)]'
                    }`}
                  >
                    {isDone ? (
                      <CheckCircle2 className="h-3.5 w-3.5" />
                    ) : (
                      <span className="font-mono">{index}</span>
                    )}
                    <span>{stepLabel}</span>
                  </span>
                </React.Fragment>
              )
            })}
          </div>
        </div>

        {/* 正文 */}
        {cameras.length === 0 ? (
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
              onClick={() => {
                onClose()
                onGoToCameras()
              }}
              className="mt-4 flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90"
            >
              <Plus className="h-3.5 w-3.5" />
              <span>{t('goToCameras', { defaultValue: '前往设备管理' })}</span>
            </button>
          </div>
        ) : (
          <form onSubmit={handleSubmit} className="flex min-h-0 flex-1 flex-col">
            <div className="grid min-h-0 flex-1 md:grid-cols-[172px_minmax(0,1fr)]">
              <aside className="hidden border-r border-[var(--border)] bg-[var(--bg-secondary)]/35 p-5 md:block">
                <span className="font-mono text-[10px] tracking-[0.18em] text-[var(--text-muted)] uppercase">
                  {t('wizard.progressLabel', { defaultValue: '创建流程' })}
                </span>
                <div className="mt-5 space-y-3">
                  <div
                    aria-current={step === 1 ? 'step' : undefined}
                    className={`relative flex items-start gap-2.5 text-xs ${
                      step === 1 ? 'text-[var(--text-primary)]' : 'text-[var(--text-muted)]'
                    }`}
                  >
                    <span
                      className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-full border font-mono text-[10px] font-bold ${
                        step === 1
                          ? 'border-[var(--accent)] bg-[var(--accent)] text-white'
                          : 'border-emerald-500/50 bg-emerald-500/10 text-emerald-500'
                      }`}
                    >
                      {step > 1 ? <CheckCircle2 className="h-3.5 w-3.5" /> : '1'}
                    </span>
                    <span className="pt-1 font-semibold">
                      {t('wizard.stepChannel', { defaultValue: '选择通道' })}
                    </span>
                  </div>
                  <div className="ml-3 h-5 border-l border-dashed border-[var(--border-strong)]" />
                  <div
                    aria-current={step === 2 ? 'step' : undefined}
                    className={`relative flex items-start gap-2.5 text-xs ${
                      step === 2 ? 'text-[var(--text-primary)]' : 'text-[var(--text-muted)]'
                    }`}
                  >
                    <span
                      className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-full border font-mono text-[10px] font-bold ${
                        step === 2
                          ? 'border-[var(--accent)] bg-[var(--accent)] text-white'
                          : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-muted)]'
                      }`}
                    >
                      2
                    </span>
                    <span className="pt-1 font-semibold">
                      {t('wizard.stepAlgorithm', { defaultValue: '选择算法' })}
                    </span>
                  </div>
                </div>
                <p className="mt-8 text-[11px] leading-relaxed text-[var(--text-muted)]">
                  {t('wizard.subtitle', {
                    defaultValue: '绑定通道后选择算法，空间防区在布防工作台内绘制。',
                  })}
                </p>
              </aside>
              <div className="min-h-0 space-y-4 overflow-y-auto p-5 pt-4 text-xs">
                {step === 1 ? (
                  <>
                    {/* 通道选择：卡片网格，直接呈现分辨率/编码/占用状态 */}
                    <div>
                      <label className="mb-1.5 block font-semibold text-[var(--text-primary)]">
                        {t('selectChannel', { defaultValue: '选择摄像头通道' })}
                        <span className="ml-1 text-rose-500">*</span>
                      </label>
                      <div className="grid max-h-56 grid-cols-1 gap-2 overflow-y-auto pr-0.5 sm:grid-cols-2">
                        {cameras.map((cam) => {
                          const isSelected = cam.cameraId === selectedCameraId
                          const hasTask = existingCameraIdsWithTasks.has(cam.cameraId)
                          return (
                            <button
                              key={cam.cameraId}
                              type="button"
                              onClick={() => handleSelectCamera(cam)}
                              disabled={isSubmitting}
                              aria-pressed={isSelected}
                              className={`flex flex-col gap-1 rounded-xl border p-2.5 text-left transition-all ${
                                isSelected
                                  ? 'border-[var(--accent)] bg-[var(--accent-soft)] shadow-xs'
                                  : 'border-[var(--border)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]'
                              }`}
                            >
                              <span className="flex items-center justify-between gap-2">
                                <span className="flex min-w-0 items-center gap-1.5">
                                  <Video
                                    className={`h-3.5 w-3.5 shrink-0 ${
                                      isSelected
                                        ? 'text-[var(--accent)]'
                                        : 'text-[var(--text-muted)]'
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
                              <span className="flex items-center gap-1.5 font-mono text-[10px] text-[var(--text-muted)]">
                                <span>
                                  {cam.lastWidth && cam.lastHeight
                                    ? `${cam.lastWidth}×${cam.lastHeight}`
                                    : '—'}
                                </span>
                                <span>·</span>
                                <span className="text-cyan-500">
                                  {cam.lastCodec?.toUpperCase() || 'H264'}
                                </span>
                                {hasTask && (
                                  <>
                                    <span>·</span>
                                    <span className="text-amber-500">
                                      {t('alreadyConfiguredTag', {
                                        defaultValue: '· [已配置任务]',
                                      })}
                                    </span>
                                  </>
                                )}
                              </span>
                            </button>
                          )
                        })}
                      </div>
                      {selectedCam && existingCameraIdsWithTasks.has(selectedCam.cameraId) && (
                        <p className="mt-1.5 text-[11px] text-amber-500">
                          {t('alreadyHasTask', {
                            defaultValue: '(该通道已有任务，保存将覆盖更新)',
                          })}
                        </p>
                      )}
                    </div>

                    {/* 任务名称 */}
                    <div>
                      <label
                        htmlFor="create-task-name"
                        className="mb-1.5 block font-semibold text-[var(--text-primary)]"
                      >
                        {t('taskName', { defaultValue: '任务名称' })}
                        <span className="ml-1 text-rose-500">*</span>
                      </label>
                      <input
                        id="create-task-name"
                        type="text"
                        value={taskName}
                        onChange={(e) => setTaskName(e.target.value)}
                        placeholder={t('taskNamePlaceholder', {
                          defaultValue: '例如：周界入侵防护 - 库房正门',
                        })}
                        disabled={isSubmitting}
                        className="w-full rounded-[6px] border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]"
                      />
                    </div>

                    {/* 分析码流来源 */}
                    <StreamModeSelector
                      value={streamMode}
                      onChange={setStreamMode}
                      disabled={isSubmitting}
                      labelClassName="mb-1.5 block font-semibold text-[var(--text-primary)]"
                    />

                    {/* 立即布防 */}
                    <div className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3">
                      <div>
                        <span className="font-semibold text-[var(--text-primary)]">
                          {t('enableArmImmediately', { defaultValue: '创建后立即启动布防' })}
                        </span>
                        <p className="mt-0.5 text-[11px] text-[var(--text-muted)]">
                          {t('enableArmImmediatelyDesc', {
                            defaultValue:
                              '开启后系统将启动该路摄像头的分析码流解码并在后台调度 NPU 规则判定。',
                          })}
                        </p>
                      </div>
                      <button
                        type="button"
                        onClick={() => setDesiredEnabled(!desiredEnabled)}
                        disabled={isSubmitting}
                        aria-pressed={desiredEnabled}
                        aria-label={t('enableArmImmediately', {
                          defaultValue: '创建后立即启动布防',
                        })}
                        className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full transition-colors ${
                          desiredEnabled ? 'bg-[var(--accent)]' : 'bg-zinc-600'
                        }`}
                      >
                        <span
                          className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow-lg transition-transform ${
                            desiredEnabled ? 'translate-x-4' : 'translate-x-0.5'
                          }`}
                        />
                      </button>
                    </div>
                  </>
                ) : (
                  <>
                    {/* 算法选择：卡片单选 */}
                    <div>
                      <div className="mb-1.5 flex items-center justify-between">
                        <label className="font-semibold text-[var(--text-primary)]">
                          {t('algorithm', { defaultValue: '分析算法' })}
                        </label>
                        {selectedAlgo && activeVersion && (
                          <span className="font-mono text-[10px] text-[var(--text-muted)]">
                            {activeVersion.platformId} · v{activeVersion.version}
                          </span>
                        )}
                      </div>
                      {availableAlgorithms.length === 0 ? (
                        <p className="rounded-xl border border-dashed border-[var(--border)] bg-[var(--bg-surface)] p-3 text-[11px] text-[var(--text-muted)]">
                          {t('algorithmFallback', {
                            defaultValue: '暂无可选算法包，创建后将使用系统默认算法。',
                          })}
                        </p>
                      ) : (
                        <div className="grid max-h-44 grid-cols-1 gap-2 overflow-y-auto pr-0.5 sm:grid-cols-2">
                          {availableAlgorithms.map((algo) => {
                            const isSelected = algo.algorithmId === selectedAlgorithmId
                            const version =
                              algo.versions.find((v) => v.isActive) || algo.versions[0]
                            const classCount = extractTargetClasses(version).length
                            return (
                              <button
                                key={algo.algorithmId}
                                type="button"
                                onClick={() => setSelectedAlgorithmId(algo.algorithmId)}
                                disabled={isSubmitting}
                                aria-pressed={isSelected}
                                className={`flex flex-col gap-1 rounded-xl border p-2.5 text-left transition-all ${
                                  isSelected
                                    ? 'border-[var(--accent)] bg-[var(--accent-soft)] shadow-xs'
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
                                <span className="flex items-center gap-1.5 font-mono text-[10px] text-[var(--text-muted)]">
                                  <span>{algo.algorithmId}</span>
                                  {classCount > 0 && (
                                    <>
                                      <span>·</span>
                                      <span>{classCount} 类</span>
                                    </>
                                  )}
                                </span>
                              </button>
                            )
                          })}
                        </div>
                      )}
                    </div>

                    {/* 算力预算：FPS */}
                    <div className="space-y-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/50 p-3">
                      <div className="flex items-center justify-between">
                        <span className="font-semibold text-[var(--text-primary)]">
                          {t('wizard.inferBudget', { defaultValue: '推理算力预算' })}
                        </span>
                        <span className="font-mono text-[11px] font-bold text-[var(--accent)]">
                          {analysisFps} FPS
                        </span>
                      </div>
                      <div className="grid grid-cols-4 gap-1.5 font-mono text-[11px]">
                        {FPS_PRESETS.map((fps) => (
                          <button
                            key={fps}
                            type="button"
                            onClick={() => setAnalysisFps(fps)}
                            disabled={isSubmitting}
                            aria-pressed={analysisFps === fps}
                            className={`rounded-lg border py-1.5 font-semibold transition-all ${
                              analysisFps === fps
                                ? 'border-[var(--accent)] bg-[var(--accent-soft)] text-[var(--accent)]'
                                : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--border-strong)]'
                            }`}
                          >
                            {fps}
                          </button>
                        ))}
                      </div>
                    </div>

                    {/* 告警置信度 */}
                    <div className="space-y-1.5">
                      <div className="flex items-center justify-between">
                        <span className="font-semibold text-[var(--text-primary)]">
                          {t('studio.confidenceThreshold', { defaultValue: '告警置信度阈值' })}
                        </span>
                        <span className="font-mono text-[11px] font-bold text-[var(--accent)]">
                          {(confidence * 100).toFixed(0)}%
                        </span>
                      </div>
                      <input
                        type="range"
                        min={0}
                        max={1}
                        step={0.05}
                        value={confidence}
                        disabled={isSubmitting}
                        aria-label={t('studio.confidenceThreshold', {
                          defaultValue: '告警置信度阈值',
                        })}
                        onChange={(e) => setConfidence(parseFloat(e.target.value))}
                        className="w-full cursor-pointer accent-[var(--accent)]"
                      />
                    </div>

                    {/* 警戒目标类别 */}
                    {availableClasses.length > 0 && (
                      <div className="space-y-1.5">
                        <div className="flex items-center justify-between">
                          <span className="font-semibold text-[var(--text-primary)]">
                            {t('studio.targetClasses', { defaultValue: '警戒目标类别' })}
                          </span>
                          <div className="flex items-center gap-1.5 text-[10px]">
                            <span className="font-mono text-[var(--text-muted)]">
                              {configuredClassCount}/{availableClasses.length}
                            </span>
                            <button
                              type="button"
                              onClick={() => setTargetClasses([...availableClasses])}
                              className="text-[var(--accent)] hover:underline"
                            >
                              {t('studio.selectAll', { defaultValue: '全选' })}
                            </button>
                            <span className="text-[var(--text-muted)]">|</span>
                            <button
                              type="button"
                              onClick={() => setTargetClasses([])}
                              className="text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                            >
                              {t('studio.clearAll', { defaultValue: '清空' })}
                            </button>
                          </div>
                        </div>
                        <div className="flex max-h-24 flex-wrap gap-1 overflow-y-auto">
                          {availableClasses.map((cls) => {
                            const isSelected = targetClasses.includes(cls)
                            return (
                              <button
                                key={cls}
                                type="button"
                                onClick={() =>
                                  setTargetClasses((prev) =>
                                    isSelected ? prev.filter((c) => c !== cls) : [...prev, cls],
                                  )
                                }
                                disabled={isSubmitting}
                                aria-pressed={isSelected}
                                className={`rounded-lg border px-2 py-0.5 text-[11px] transition-all ${
                                  isSelected
                                    ? 'border-[var(--accent)] bg-[var(--accent)] text-white'
                                    : 'border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--border-strong)]'
                                }`}
                              >
                                {getLocalizedClassName(cls, i18n.language)}
                              </button>
                            )
                          })}
                        </div>
                      </div>
                    )}

                    <p className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/50 p-2.5 text-[10px] leading-relaxed text-[var(--text-muted)]">
                      {t('wizard.moreParamsHint', {
                        defaultValue:
                          '更多算法参数可在创建后进入布防工作台，点击算法方块上的齿轮按钮继续微调。',
                      })}
                    </p>
                  </>
                )}

                {/* 错误提示 */}
                {errorMsg && (
                  <div className="flex items-center gap-2 rounded-xl border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-500">
                    <AlertCircle className="h-4 w-4 shrink-0" />
                    <span>{errorMsg}</span>
                  </div>
                )}
              </div>
            </div>

            {/* 底部操作栏 */}
            <div className="flex shrink-0 items-center justify-between gap-3 border-t border-[var(--border)] p-4">
              <button
                type="button"
                onClick={step === 1 ? onClose : () => setStep(1)}
                disabled={isSubmitting}
                className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-semibold text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
              >
                {step === 1 ? (
                  <span>{tc('actions.cancel', { defaultValue: '取消' })}</span>
                ) : (
                  <>
                    <ArrowLeft className="h-3.5 w-3.5" />
                    <span>{t('wizard.back', { defaultValue: '上一步' })}</span>
                  </>
                )}
              </button>

              {step === 1 ? (
                <button
                  type="button"
                  onClick={handleGoToStepTwo}
                  disabled={isSubmitting}
                  className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95 disabled:opacity-50"
                >
                  <span>{t('wizard.next', { defaultValue: '下一步' })}</span>
                  <ArrowRight className="h-3.5 w-3.5" />
                </button>
              ) : (
                <button
                  type="submit"
                  disabled={isSubmitting}
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
              )}
            </div>
          </form>
        )}
      </div>
    </div>
  )
}
